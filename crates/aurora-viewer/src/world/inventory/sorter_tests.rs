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
            ..Default::default()
        },
        Rule {
            words: "".into(),
            folder: "Ignored".into(),
            ..Default::default()
        },
    ];
    assert_eq!(
        destination("Black leather CHAIR", 6, 0, &rules),
        Some(vec!["Home".to_owned(), "Black chairs".to_owned()])
    );
    assert_eq!(destination("White chair", 6, 0, &rules), Some(vec!["Furniture".to_owned()]));
}

fn path_of(to: &Dest) -> String {
    match to {
        Dest::Path(path) => path.join("/"),
        Dest::Folder(id) => format!("#{id}"),
        Dest::PreTrash => "pre-trash".to_owned(),
    }
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
    let moved: Vec<_> = plan.moves.iter().map(|m| (m.name.as_str(), path_of(&m.to))).collect();
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
    let Step::Change(change, taken) = step(&mut inv, &prefs.base_name(), &mut queue, &HashSet::new(), &HashSet::new(), &mut 0) else {
        panic!("a mutation")
    };
    assert_eq!(taken.len(), 5);
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
        step(&mut inv, &prefs.base_name(), &mut queue, &HashSet::new(), &HashSet::new(), &mut 0),
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
    while let Step::Change(change, taken) = step(&mut inv, &prefs.base_name(), &mut queue, &HashSet::new(), &HashSet::new(), &mut 0) {
        batches.push(taken.len());
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
        step(&mut inv, &prefs.base_name(), &mut queue, &HashSet::new(), &HashSet::new(), &mut 0),
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
    while let Step::Change(change, _) = step(&mut inv, &prefs.base_name(), &mut queue, &HashSet::new(), &HashSet::new(), &mut 0) {
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

#[test]
fn a_rule_matches_every_word_any_word_or_the_exact_phrase() {
    let rule = |words: &str, mode: Match| Rule {
        words: words.into(),
        mode,
        folder: "Mine".into(),
        ..Default::default()
    };
    let name = words("Black leather CHAIR - grey");
    assert!(rule("chair black", Match::All).matches(&name, None));
    assert!(!rule("chair white", Match::All).matches(&name, None));
    assert!(rule("chair white", Match::Any).matches(&name, None));
    assert!(!rule("sofa white", Match::Any).matches(&name, None));
    assert!(rule("leather chair", Match::Phrase).matches(&name, None));
    assert!(!rule("chair leather", Match::Phrase).matches(&name, None));
    // a switched-off or incomplete rule takes nothing
    let off = Rule {
        enabled: false,
        ..rule("chair", Match::All)
    };
    assert!(!off.usable());
    assert!(
        !Rule {
            folder: " / ".into(),
            ..rule("chair", Match::All)
        }
        .usable()
    );
}

#[test]
fn a_rule_can_send_entries_to_one_of_the_users_own_folders() {
    let mut inv = fixture(&["Black leather chair", "Velvet skirt Reborn"]);
    let home = id(70);
    inv.folders.insert(home, folder(home, id(1), "Home", -1));
    inv.folders.get_mut(&id(1)).expect("root").children.push(home);
    let prefs = Preferences {
        rules: vec![Rule {
            words: "chair".into(),
            target: Some(home),
            ..Default::default()
        }],
        ..Default::default()
    };
    let plan = plan(&inv, id(OBJECTS), &prefs, &HashSet::new(), &HashSet::new());
    let chair = plan.moves.iter().find(|m| m.id == id(100)).expect("the chair moves");
    assert_eq!(chair.to, Dest::Folder(home));
    assert_eq!(chair.label, "My Inventory / Home");
    let mut queue: VecDeque<Move> = plan.moves.into();
    while let Step::Change(change, _) = step(&mut inv, &prefs.base_name(), &mut queue, &HashSet::new(), &HashSet::new(), &mut 0) {
        // the chosen folder is used as it is: nothing is made inside it
        assert!(
            !change
                .operations
                .iter()
                .any(|op| matches!(op, Operation::Folder { parent, .. } if *parent == home))
        );
        actions::demo_mutate(&mut inv, id(AGENT), *change).expect("demo server");
    }
    assert_eq!(inv.folders[&home].items, [id(100)]);
    // already there: a second plan leaves it alone
    assert!(super::plan(&inv, home, &prefs, &HashSet::new(), &HashSet::new()).moves.is_empty());
    // a rule folder in the Trash or gone cannot receive anything
    let gone = Preferences {
        rules: vec![Rule {
            words: "skirt".into(),
            target: Some(id(11)),
            ..Default::default()
        }],
        ..Default::default()
    };
    let plan = plan_entries(&inv, &[id(101)], &gone, &HashSet::new(), &HashSet::new());
    assert!(plan.moves.is_empty());
    assert_eq!(plan.unknown, ["Velvet skirt Reborn"]);
}

#[test]
fn chosen_entries_sort_alone_wherever_they_are() {
    let inv = fixture(&["Velvet skirt Reborn", "Lace bra Reborn", "Mystery box"]);
    let prefs = Preferences::default();
    let plan = plan_entries(&inv, &[id(100), id(102)], &prefs, &HashSet::new(), &HashSet::new());
    let moved: Vec<_> = plan.moves.iter().map(|m| (m.name.as_str(), path_of(&m.to))).collect();
    assert_eq!(moved, [("Velvet skirt Reborn", "Reborn/Skirts".to_owned())]);
    assert_eq!(plan.unknown, ["Mystery box"]);
}

#[test]
fn heads_huds_vehicles_and_pets_have_their_own_folders() {
    assert_eq!(path("LeLutka Head Avalon EvoX").as_deref(), Some("Heads"));
    assert_eq!(path("Body HUD v3").as_deref(), Some("HUDs"));
    assert_eq!(path("Speedster motorbike").as_deref(), Some("Vehicles"));
    assert_eq!(path("Fluffy kitten companion").as_deref(), Some("Pets"));
}

#[test]
fn squish_versions_always_get_their_own_folder() {
    let to = |name: &str| match target(name, 6, 0, None, &[], false) {
        Some(Dest::Path(path)) => path.join("/"),
        other => format!("{other:?}"),
    };
    assert_eq!(to("Kitty - Hex Dress (Reborn Squish) - FATPACK"), "Reborn/Dresses/Squish");
    assert_eq!(to("Reborn bra black sq"), "Reborn/Lingerie/Squish");
    assert_eq!(to("Lace bra Reborn - Squeeze"), "Reborn/Lingerie/Squish");
    assert_eq!(to("Reborn bra black"), "Reborn/Lingerie");
}

#[test]
fn the_store_is_read_from_the_creators_tag() {
    assert_eq!(store("[Birch Hollow] + Tucker Round Gothic Table").as_deref(), Some("Birch Hollow"));
    assert_eq!(store("*Neverlace* Neon Heart Shelf").as_deref(), Some("Neverlace"));
    assert_eq!(store(".VX. Vixen Dress").as_deref(), Some("VX"));
    assert_eq!(store("Kitty - Hex Dress (Reborn Squish) - FATPACK").as_deref(), Some("Kitty"));
    assert_eq!(store("rvn // Platform Boots").as_deref(), Some("rvn"));
    // a body tag or a product before the dash is no store
    assert_eq!(store("[REBORN] Denim Skirt"), None);
    assert_eq!(store("Reborn - Waifu Boobs"), None);
    assert_eq!(store("Velvet Sofa Modular - PG"), None);
    assert_eq!(store("Mystery box"), None);
    let to = |name: &str| match target(name, 6, 0, None, &[], true) {
        Some(Dest::Path(path)) => path.join("/"),
        other => format!("{other:?}"),
    };
    assert_eq!(to("[Birch Hollow] + Semaj Pillow Couch (black)"), "Furniture/Birch Hollow");
    assert_eq!(to("*Neverlace* Mystery Crate"), "Stores/Neverlace");
}

#[test]
fn an_items_type_wins_over_furniture_words_in_its_name() {
    assert_eq!(
        destination("Bar bottom texture (without chair shadows)", 0, 0, &[]),
        Some(vec!["Textures".to_owned()])
    );
    assert_eq!(
        destination("Chair sit animation", 20, 0, &[]),
        Some(vec!["Animations & poses".to_owned()])
    );
    // an object is still read from its name
    assert_eq!(destination("Black leather chair", 6, 0, &[]), Some(vec!["Furniture".to_owned()]));
}

#[test]
fn rules_read_words_only_and_an_old_dated_rule_takes_nothing() {
    assert_eq!(date_start("2020"), Some(1_577_836_800));
    assert_eq!(date_start("2020-06"), Some(1_590_969_600));
    assert_eq!(date_start("2020/06/15"), Some(1_592_179_200));
    assert_eq!(date_start("June 2020"), None);
    assert_eq!(date_start("2020-13"), None);
    // a rule saved by the first version with a date and no words: dates are
    // Clean up's now, so it is incomplete rather than taking everything
    let dated = Rule {
        before: "2020".into(),
        pre_trash: true,
        ..Default::default()
    };
    assert!(!dated.usable());
    let inv = fixture(&["Old freebie dress"]);
    let prefs = Preferences {
        rules: vec![dated],
        ..Default::default()
    };
    let plan = plan(&inv, id(OBJECTS), &prefs, &HashSet::new(), &HashSet::new());
    assert!(plan.moves.iter().all(|m| m.to != Dest::PreTrash));
    // old preferences with the date still load
    let old: Rule = serde_json::from_str(r#"{"words":"chair","folder":"Home","before":"2019","variants":true}"#).expect("old rule");
    assert_eq!(old.words, "chair");
}

#[test]
fn excluded_folders_the_trash_and_marketplace_listings_are_never_touched() {
    let mut inv = fixture(&["Velvet skirt Reborn", "Lace bra Reborn"]);
    let listings = id(80);
    inv.folders.insert(listings, folder(listings, id(1), "Marketplace Listings", 53));
    inv.folders.insert(id(81), folder(id(81), listings, "Reborn Skirt listing", -1));
    inv.folders.get_mut(&listings).expect("listings").children.push(id(81));
    inv.folders.get_mut(&id(1)).expect("root").children.push(listings);
    assert!(!source_allowed(&inv, listings));
    assert!(!source_allowed(&inv, id(81)));
    assert!(!target_allowed(&inv, id(81)));
    assert!(!target_allowed(&inv, id(11)));
    let picked = plan_entries(&inv, &[id(81)], &Preferences::default(), &HashSet::new(), &HashSet::new());
    assert!(picked.moves.is_empty());
    // an excluded Objects keeps everything
    let prefs = Preferences {
        excluded: vec![id(OBJECTS)],
        ..Default::default()
    };
    let plan = plan(&inv, id(OBJECTS), &prefs, &HashSet::new(), &HashSet::new());
    assert!(plan.moves.is_empty());
    assert_eq!(plan.kept, 2);
}

#[test]
fn the_whole_inventory_reads_objects_and_moves_root_deliveries_whole() {
    let mut inv = fixture(&["Velvet skirt Reborn"]);
    // a loose item at the root, a user folder and its contents
    let home = id(90);
    inv.folders.insert(home, folder(home, id(1), "My chairs", -1));
    inv.folders.get_mut(&id(1)).expect("root").children.push(home);
    let mut loose = inv.items[&id(100)].clone();
    loose.id = id(150);
    loose.parent = id(1);
    loose.name = "Denim skirt Legacy".into();
    let mut kept = loose.clone();
    kept.id = id(151);
    kept.parent = home;
    kept.name = "Black leather chair".into();
    inv.add_items(vec![loose, kept]);
    // a delivery folder in Objects moves whole, its contents stay inside
    let fatpack = id(95);
    inv.folders
        .insert(fatpack, folder(fatpack, id(OBJECTS), "Kitty - Hex Dress - FATPACK", -1));
    inv.folders.get_mut(&id(OBJECTS)).expect("objects").children.push(fatpack);
    let mut part = inv.items[&id(100)].clone();
    part.id = id(152);
    part.parent = fatpack;
    part.name = "Kitty - Hex Dress HUD".into();
    inv.add_items(vec![part]);
    let prefs = Preferences::default();
    assert_eq!(whole_sources(&inv, &prefs), [id(1), id(OBJECTS)]);
    let plan = plan_whole(&inv, &prefs, &HashSet::new(), &HashSet::new());
    let mut moved: Vec<_> = plan.moves.iter().map(|m| m.id).collect();
    moved.sort();
    // "My chairs", the user's folder lying at the root with a name that
    // reads as furniture and no folder inside, moves whole like a delivery
    assert_eq!(moved, [home, fatpack, id(100), id(150)]);
    // excluding Objects leaves the root's loose item and its delivery
    let prefs = Preferences {
        excluded: vec![id(OBJECTS)],
        ..Default::default()
    };
    let plan = plan_whole(&inv, &prefs, &HashSet::new(), &HashSet::new());
    let mut moved: Vec<_> = plan.moves.iter().map(|m| m.id).collect();
    moved.sort();
    assert_eq!(moved, [home, id(150)]);
}

const OLD: i64 = 1_400_000_000;
const NEW: i64 = 1_700_000_000;

fn add_folder(inv: &mut Inventory, at: u128, parent: u128, name: &str) {
    inv.folders.insert(id(at), folder(id(at), id(parent), name, -1));
    inv.folders.get_mut(&id(parent)).expect("parent").children.push(id(at));
}

fn add_typed(inv: &mut Inventory, at: u128, parent: u128, name: &str, created: i64, asset_type: i32) {
    inv.add_items(vec![InvItem {
        id: id(at),
        parent: id(parent),
        name: name.into(),
        asset_type,
        inv_type: asset_type,
        owner: id(AGENT),
        created_at: created,
        ..Default::default()
    }]);
}

fn add_item(inv: &mut Inventory, at: u128, parent: u128, name: &str, created: i64) {
    add_typed(inv, at, parent, name, created, 6);
}

/// A Pre-Trash word rule taking denim, freebies, the Hex dress and anything
/// called old.
fn word_rule(skip_sorted: bool) -> Preferences {
    Preferences {
        rules: vec![Rule {
            words: "denim freebie hex old".into(),
            mode: Match::Any,
            pre_trash: true,
            skip_sorted,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn pre_trashed(plan: &Plan) -> Vec<Uuid> {
    let mut ids: Vec<_> = plan.moves.iter().filter(|m| m.to == Dest::PreTrash).map(|m| m.id).collect();
    ids.sort();
    ids
}

/// Sorted/Reborn/Skirts with an old and a new skirt, Objects/My stuff (the
/// user's own folder) with an old item, and two delivery folders in
/// Objects: one all old, one with a single new item.
fn sorted_fixture() -> Inventory {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 20, 1, "Sorted");
    add_folder(&mut inv, 21, 20, "Reborn");
    add_folder(&mut inv, 22, 21, "Skirts");
    add_item(&mut inv, 200, 22, "Denim skirt Reborn", OLD);
    add_item(&mut inv, 201, 22, "Velvet skirt Reborn", NEW);
    add_folder(&mut inv, 30, OBJECTS, "My stuff");
    add_item(&mut inv, 300, 30, "Freebie lamp", OLD);
    add_item(&mut inv, 301, 30, "Gift lamp", NEW);
    // a folder of its own: My stuff is the user's arrangement, not a product
    add_folder(&mut inv, 31, 30, "Lamps");
    add_folder(&mut inv, 40, OBJECTS, "Lumen - Hex Dress - FATPACK");
    add_item(&mut inv, 400, 40, "Lumen - Hex Dress HUD", OLD);
    add_item(&mut inv, 401, 40, "Lumen - Hex Dress (Reborn)", OLD);
    add_folder(&mut inv, 50, OBJECTS, "Lumen - Wrap Skirt - FATPACK");
    add_item(&mut inv, 500, 50, "Lumen - Wrap Skirt HUD", OLD);
    add_item(&mut inv, 501, 50, "Lumen - Wrap Skirt (Reborn)", NEW);
    inv.generation += 1;
    inv
}

#[test]
fn pre_trash_rules_reach_sorted_items_and_the_users_own_folders() {
    let inv = sorted_fixture();
    let plan = plan(&inv, id(OBJECTS), &word_rule(false), &HashSet::new(), &HashSet::new());
    let trashed = pre_trashed(&plan);
    // the denim skirt already sorted, the freebie in the user's folder, and
    // the Hex dress delivery whole (named by the rule, never opened)
    assert_eq!(trashed, [id(40), id(200), id(300)]);
    for kept in [201, 301, 400, 401, 50, 500, 501] {
        assert!(!trashed.contains(&id(kept)), "{kept} stays");
    }
    // Pre-Trash comes first in the preview
    assert!(plan.groups().first().is_some_and(|(label, _)| label.ends_with("Pre-Trash")));
}

#[test]
fn everywhere_except_sorted_leaves_the_sorted_tree_alone() {
    let inv = sorted_fixture();
    let plan = plan(&inv, id(OBJECTS), &word_rule(true), &HashSet::new(), &HashSet::new());
    let trashed = pre_trashed(&plan);
    assert!(!trashed.contains(&id(200)), "sorted skirt stays");
    assert!(trashed.contains(&id(300)) && trashed.contains(&id(40)));
    // the same rule run on the Sorted folder itself takes nothing there
    let plan = super::plan(&inv, id(22), &word_rule(true), &HashSet::new(), &HashSet::new());
    assert!(pre_trashed(&plan).is_empty());
    let plan = super::plan(&inv, id(22), &word_rule(false), &HashSet::new(), &HashSet::new());
    assert_eq!(pre_trashed(&plan), [id(200)]);
}

/// The never-touched folders around [`sorted_fixture`]: an old item in the
/// Trash, an old listing in Marketplace Listings.
fn guarded_fixture() -> Inventory {
    let mut inv = sorted_fixture();
    add_item(&mut inv, 110, 11, "Old thing in the trash", OLD);
    inv.folders.insert(id(80), folder(id(80), id(1), "Marketplace Listings", 53));
    inv.folders.get_mut(&id(1)).expect("root").children.push(id(80));
    add_folder(&mut inv, 81, 80, "Old listing");
    add_item(&mut inv, 810, 81, "Old listed dress", OLD);
    inv
}

#[test]
fn pre_trash_never_reads_the_trash_marketplace_or_excluded_folders() {
    let inv = guarded_fixture();
    let mut prefs = word_rule(false);
    prefs.excluded = vec![id(30)];
    let whole = plan_whole(&inv, &prefs, &HashSet::new(), &HashSet::new());
    let trashed = pre_trashed(&whole);
    for untouched in [110, 81, 810, 30, 300] {
        assert!(!trashed.contains(&id(untouched)), "{untouched} untouched");
    }
    // the rest of My Inventory is read at any depth
    assert!(trashed.contains(&id(200)) && trashed.contains(&id(40)));
    // a locked folder and a worn item stay too
    let protected: HashSet<Uuid> = [id(40)].into();
    let worn: HashSet<Uuid> = [id(200)].into();
    let trashed = pre_trashed(&plan_whole(&inv, &prefs, &protected, &worn));
    assert!(!trashed.contains(&id(40)) && !trashed.contains(&id(200)));
}

#[test]
fn unloaded_folders_are_listed_for_the_window_to_request() {
    let mut inv = sorted_fixture();
    inv.folders.get_mut(&id(21)).expect("Reborn").state = FetchState::Unknown;
    let plan = plan(&inv, id(OBJECTS), &word_rule(false), &HashSet::new(), &HashSet::new());
    assert!(plan.unfetched.contains(&id(21)));
    assert!(!pre_trashed(&plan).contains(&id(200)));
    // without a Pre-Trash rule nothing is read beyond the direct contents
    let plan = super::plan(&inv, id(OBJECTS), &Preferences::default(), &HashSet::new(), &HashSet::new());
    assert!(plan.unfetched.is_empty());
}

/// 15 November 2026, the day the Clean up tests count back from.
fn today() -> i64 {
    date_start("2026-11-15").expect("date")
}

/// Everything acquired before January 2020, of every kind.
fn before_2020() -> Cleanup {
    Cleanup {
        age: Age::Custom,
        year: 2020,
        month: 1,
        ..Default::default()
    }
}

fn cleaned(plan: &CleanupPlan) -> Vec<Uuid> {
    let mut ids: Vec<_> = plan.rows.iter().map(|r| r.id).collect();
    ids.sort();
    ids
}

#[test]
fn the_date_chips_count_back_from_today() {
    let cutoff = |age: Age, now: i64| Cleanup { age, ..Default::default() }.cutoff(now);
    assert_eq!(cutoff(Age::OneYear, today()), date_start("2025-11-15"));
    assert_eq!(cutoff(Age::TwoYears, today()), date_start("2024-11-15"));
    assert_eq!(cutoff(Age::ThreeYears, today()), date_start("2023-11-15"));
    assert_eq!(cutoff(Age::FiveYears, today()), date_start("2021-11-15"));
    // the middle of a day counts from that day; 29 February falls back
    assert_eq!(cutoff(Age::OneYear, today() + 50_000), date_start("2025-11-15"));
    assert_eq!(
        cutoff(Age::OneYear, date_start("2028-02-29").expect("leap")),
        date_start("2027-02-28")
    );
    let custom = Cleanup {
        age: Age::Custom,
        year: 2020,
        month: 3,
        ..Default::default()
    };
    assert_eq!(custom.cutoff(today()), date_start("2020-03"));
}

#[test]
fn clean_up_takes_old_items_everywhere_and_never_splits_a_delivery() {
    let inv = sorted_fixture();
    let prefs = Preferences::default();
    let plan = plan_cleanup(&inv, &prefs, &before_2020(), id(OBJECTS), today(), &HashSet::new(), &HashSet::new());
    // the old sorted skirt, the old item of the user's folder (alone), and
    // the delivery whose items are all old, whole
    assert_eq!(cleaned(&plan), [id(40), id(200), id(300)]);
    let folder = plan.rows.iter().find(|r| r.id == id(40)).expect("folder row");
    assert!(folder.folder && folder.items == 2);
    // the mixed delivery stays whole, its old HUD included
    assert!(!cleaned(&plan).contains(&id(500)) && !cleaned(&plan).contains(&id(50)));
    // "This folder" reads Objects only, "Include Sorted" off skips Sorted
    let here = Cleanup {
        whole: false,
        ..before_2020()
    };
    assert_eq!(
        cleaned(&plan_cleanup(
            &inv,
            &prefs,
            &here,
            id(OBJECTS),
            today(),
            &HashSet::new(),
            &HashSet::new()
        )),
        [id(40), id(300)]
    );
    let unsorted = Cleanup {
        include_sorted: false,
        ..before_2020()
    };
    assert_eq!(
        cleaned(&plan_cleanup(
            &inv,
            &prefs,
            &unsorted,
            id(OBJECTS),
            today(),
            &HashSet::new(),
            &HashSet::new()
        )),
        [id(40), id(300)]
    );
    // unchecked rows stay
    let skipped: HashSet<Uuid> = [id(200)].into();
    let moves = plan.moves(&skipped);
    assert_eq!(moves.len(), 2);
    assert!(moves.iter().all(|m| m.to == Dest::PreTrash && m.id != id(200)));
}

#[test]
fn clean_up_filters_by_type() {
    let mut inv = sorted_fixture();
    add_typed(&mut inv, 302, 30, "Old store landmark", OLD, 3);
    add_typed(&mut inv, 303, 30, "Old notecard", OLD, 7);
    let landmarks = Cleanup {
        kinds: vec![Kind::Landmarks],
        ..before_2020()
    };
    let plan = plan_cleanup(
        &inv,
        &Preferences::default(),
        &landmarks,
        id(1),
        today(),
        &HashSet::new(),
        &HashSet::new(),
    );
    assert_eq!(cleaned(&plan), [id(302)]);
    // a delivery is taken whole only when every item in it is of a chosen kind
    let objects = Cleanup {
        kinds: vec![Kind::Objects],
        ..before_2020()
    };
    let plan = plan_cleanup(
        &inv,
        &Preferences::default(),
        &objects,
        id(1),
        today(),
        &HashSet::new(),
        &HashSet::new(),
    );
    assert_eq!(cleaned(&plan), [id(40), id(200), id(300)]);
    let none = Cleanup {
        kinds: Vec::new(),
        ..before_2020()
    };
    assert!(
        plan_cleanup(
            &inv,
            &Preferences::default(),
            &none,
            id(1),
            today(),
            &HashSet::new(),
            &HashSet::new()
        )
        .rows
        .is_empty()
    );
}

#[test]
fn clean_up_never_touches_guarded_folders_worn_items_or_links() {
    let mut inv = guarded_fixture();
    add_typed(&mut inv, 304, 30, "Old link", OLD, 24);
    let mut prefs = Preferences::default();
    let plan = plan_cleanup(&inv, &prefs, &before_2020(), id(1), today(), &HashSet::new(), &HashSet::new());
    let rows = cleaned(&plan);
    for untouched in [110, 80, 81, 810, 304] {
        assert!(!rows.contains(&id(untouched)), "{untouched} untouched");
    }
    assert!(rows.contains(&id(300)));
    // worn, locked and excluded stay
    let worn: HashSet<Uuid> = [id(300)].into();
    let protected: HashSet<Uuid> = [id(40)].into();
    let rows = cleaned(&plan_cleanup(&inv, &prefs, &before_2020(), id(1), today(), &protected, &worn));
    assert!(!rows.contains(&id(300)) && !rows.contains(&id(40)));
    prefs.excluded = vec![id(30)];
    let rows = cleaned(&plan_cleanup(
        &inv,
        &prefs,
        &before_2020(),
        id(1),
        today(),
        &HashSet::new(),
        &HashSet::new(),
    ));
    assert!(!rows.contains(&id(300)));
    // the Pre-Trash folder itself is never read again
    add_folder(&mut inv, 60, 1, "Pre-Trash");
    add_item(&mut inv, 600, 60, "Waiting", OLD);
    let rows = cleaned(&plan_cleanup(
        &inv,
        &Preferences::default(),
        &before_2020(),
        id(1),
        today(),
        &HashSet::new(),
        &HashSet::new(),
    ));
    assert!(!rows.contains(&id(600)) && !rows.contains(&id(60)));
}

#[test]
fn clean_up_lists_unloaded_folders_and_waits_for_them() {
    let mut inv = sorted_fixture();
    inv.folders.get_mut(&id(30)).expect("My stuff").state = FetchState::Unknown;
    let plan = plan_cleanup(
        &inv,
        &Preferences::default(),
        &before_2020(),
        id(1),
        today(),
        &HashSet::new(),
        &HashSet::new(),
    );
    assert!(plan.unfetched.contains(&id(30)));
    assert!(!cleaned(&plan).contains(&id(300)) && !cleaned(&plan).contains(&id(30)));
}

/// The path `name` (an object) goes to with a folder per store, as text.
fn sorted_as(name: &str, kind: i32, hints: &[StoreHint]) -> String {
    match target_with(name, kind, 0, None, &[], true, hints) {
        Some(Dest::Path(path)) => path.join("/"),
        other => format!("{other:?}"),
    }
}

fn cat(c: Category) -> &'static str {
    c.folder()
}

/// Names from a real inventory, as creators write them (shared by the
/// human), and where they go with a folder per store.
#[test]
fn real_creator_names_find_their_folders() {
    use Category::*;
    let stores = aurora_i18n::tr!("inventory-sorter-stores");
    let none = "None".to_owned();
    let cases: Vec<(&str, i32, String)> = vec![
        // folders: deliveries and boxes
        ("Tredente // Bento Money Envelope (Packed)", -1, format!("{stores}/Tredente")),
        ("TROLLINGAVATAR", -1, none.clone()),
        ("TsVe 1.2.5 Box", -1, none.clone()),
        (
            "uh-oh: Ali Fruity Striped Thigh Socks  - Licorice (Wear)",
            -1,
            format!("{}/uh-oh", cat(Stockings)),
        ),
        ("UnityCore Combat System V39.812", -1, none.clone()),
        ("{UE} What a S****y Attack Hud [boxed]", -1, format!("{}/UE", cat(Huds))),
        ("~JJ~ The Chameleon Halo Set", -1, format!("{}/JJ", cat(Outfits))),
        // objects
        (
            "[BS][GESTURES][1] AUSSIE MEAT PIE PACK (ADD+CLICK)",
            6,
            format!("{}/BS", cat(Gestures)),
        ),
        ("Kitoomi - Ryu Jeans - FATPACK", 6, format!("{}/Kitoomi", cat(Pants))),
        ("LeLUTKA Otto Head 4.0", 6, cat(Heads).to_owned()),
        ("OnlyPixel_Terminal_Office_High1", 6, format!("{stores}/OnlyPixel")),
        ("OnlyPx_Sign_F", 6, format!("{stores}/OnlyPx")),
        ("Object", 6, none.clone()),
        ("[Grazed] Rebellion Jacket // Fatpack", 6, format!("{}/Grazed", cat(Jackets))),
        ("Obsidian Homes ~ Eclipse (ADD)", 6, format!("{stores}/Obsidian Homes")),
        ("OnlyPixel Deposit HUD", 6, cat(Huds).to_owned()),
        ("P_Scaner_Salty", 6, none.clone()),
        ("Sera - Feelin Undone - Bom Bandages Fatpack", 6, format!("{}/Sera", cat(Tattoos))),
        // a product's HUD goes with the product
        ("[Grazed] Trauma Tee // Fatpack Hud", 6, format!("{}/Grazed", cat(Tops))),
        // LegacyM: Legacy's male body; AT, its Athletic shape
        (
            "[Grazed] Trauma Tee // Fatpack // LegacyM+AT",
            6,
            format!("Legacy Male/{}/Grazed", cat(Tops)),
        ),
        ("MadPea Willy Wrappers - Glove", 6, format!("{}/MadPea Willy Wrappers", cat(Gloves))),
        ("Emy's Birthday Invitation", 6, none.clone()),
        ("Mouth Spitter (Light Pink)", 6, none.clone()),
        // Waifus, Reborn's body mod: the mod itself is an add-on, its clothes
        // get a Waifus folder (with Squish when both)
        ("Waifu REBORN - Waifus - Addon v2.0", 6, format!("Reborn/{}", cat(BodyAddons))),
        (
            "Reborn Feat. Axolotl - Waifu Bikini - Top Slim",
            6,
            format!("Reborn/{}/Waifus", cat(Swimwear)),
        ),
        (
            "Muse - Lovesick Darling - Reborn Waifus - Bra",
            6,
            format!("Reborn/{}/Muse/Waifus", cat(Lingerie)),
        ),
        (
            "Rokins - Rihanna Top - Black - [Waifus size Squish]",
            6,
            format!("Reborn/{}/Rokins/Waifus Squish", cat(Tops)),
        ),
        // a body skin, named by its body and tone; a body mod maker
        (
            "VELOUR: The \"Ipanema Body\" for eBody Reborn (Porcelain)",
            6,
            format!("Reborn/{}/VELOUR", cat(Skins)),
        ),
        (
            "MAZE.mods - Soft Thighs V4.2 - eBody Reborn",
            6,
            format!("Reborn/{}/MAZE.mods", cat(BodyAddons)),
        ),
    ];
    let mut wrong = Vec::new();
    for (name, kind, expected) in &cases {
        let got = sorted_as(name, *kind, &[]);
        if &got != expected {
            wrong.push(format!("{name:?}: got {got:?}, expected {expected:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn store_hints_give_a_category_to_names_that_say_nothing() {
    let hints = [StoreHint {
        store: "obsidian homes".into(),
        category: Category::Furniture,
        ..Default::default()
    }];
    assert_eq!(
        sorted_as("Obsidian Homes ~ Eclipse (ADD)", 6, &hints),
        format!("{}/Obsidian Homes", Category::Furniture.folder())
    );
    // a name's own category wins over the hint
    let hints = [StoreHint {
        store: "Kitoomi".into(),
        category: Category::Furniture,
        ..Default::default()
    }];
    assert_eq!(
        sorted_as("Kitoomi - Ryu Jeans - FATPACK", 6, &hints),
        format!("{}/Kitoomi", Category::Pants.folder())
    );
    // without a folder per store, the hint still files it
    assert_eq!(
        target_with(
            "Obsidian Homes ~ Eclipse (ADD)",
            6,
            0,
            None,
            &[],
            false,
            &[StoreHint {
                store: "Obsidian Homes".into(),
                category: Category::Furniture,
                ..Default::default()
            }]
        ),
        Some(Dest::Path(vec![Category::Furniture.folder().to_owned()]))
    );
}

#[test]
fn the_sorts_type_filter_keeps_unticked_kinds_in_place() {
    let mut inv = fixture(&["Velvet skirt Reborn"]);
    add_typed(&mut inv, 120, OBJECTS, "Store landmark", NEW, 3);
    add_typed(&mut inv, 121, OBJECTS, "Read me notecard", NEW, 7);
    let all = plan(&inv, id(OBJECTS), &Preferences::default(), &HashSet::new(), &HashSet::new());
    assert_eq!(all.moves.len(), 3);
    let objects_only = Preferences {
        kinds: vec![Kind::Objects],
        ..Default::default()
    };
    let plan = plan(&inv, id(OBJECTS), &objects_only, &HashSet::new(), &HashSet::new());
    assert_eq!(plan.moves.iter().map(|m| m.id).collect::<Vec<_>>(), [id(100)]);
    assert!(plan.unknown.is_empty(), "left out, not unknown");
    // a right-click on chosen entries sorts them whatever the filter
    let picked = plan_entries(&inv, &[id(120)], &objects_only, &HashSet::new(), &HashSet::new());
    assert_eq!(picked.moves.len(), 1);
}

#[test]
fn clean_up_takes_empty_folders_when_asked() {
    let mut inv = sorted_fixture();
    add_folder(&mut inv, 70, 1, "Empty boxes");
    add_folder(&mut inv, 71, 70, "Nothing here");
    add_folder(&mut inv, 72, 30, "Also empty");
    // an empty folder Auto-sort made is taken too, never the sort folder
    add_folder(&mut inv, 23, 21, "Dresses");
    let asked = Cleanup {
        empty_folders: true,
        ..before_2020()
    };
    let rows = cleaned(&plan_cleanup(
        &inv,
        &Preferences::default(),
        &asked,
        id(1),
        today(),
        &HashSet::new(),
        &HashSet::new(),
    ));
    // the outer empty folder whole (its empty child inside it)
    assert!(rows.contains(&id(70)) && !rows.contains(&id(71)));
    assert!(rows.contains(&id(72)) && rows.contains(&id(23)));
    assert!(!rows.contains(&id(20)) && !rows.contains(&id(11)));
    // off by default
    let rows = cleaned(&plan_cleanup(
        &inv,
        &Preferences::default(),
        &before_2020(),
        id(1),
        today(),
        &HashSet::new(),
        &HashSet::new(),
    ));
    assert!(!rows.contains(&id(70)) && !rows.contains(&id(72)));
    // with no kind ticked, only the empty folders
    let only_empty = Cleanup {
        kinds: Vec::new(),
        ..asked
    };
    let rows = cleaned(&plan_cleanup(
        &inv,
        &Preferences::default(),
        &only_empty,
        id(1),
        today(),
        &HashSet::new(),
        &HashSet::new(),
    ));
    assert!(rows.contains(&id(70)) && !rows.contains(&id(300)));
}

#[test]
fn shared_settings_round_trip_without_my_folders() {
    use super::share::{Pick, Share, export, import};
    let prefs = Preferences {
        base: "Tidy".into(),
        stores: true,
        kinds: vec![Kind::Objects, Kind::Clothing],
        rules: vec![
            Rule {
                words: "denim jeans".into(),
                mode: Match::Any,
                folder: "Clothes/Denim".into(),
                ..Default::default()
            },
            Rule {
                words: "old freebie".into(),
                pre_trash: true,
                skip_sorted: true,
                ..Default::default()
            },
            Rule {
                words: "secret stash".into(),
                target: Some(id(30)),
                ..Default::default()
            },
        ],
        store_hints: vec![StoreHint {
            store: "Birch Hollow".into(),
            category: Category::Furniture,
            ..Default::default()
        }],
        excluded: vec![id(30)],
        ..Default::default()
    };
    let text = export(&prefs, &Share::all(&prefs));
    assert!(text.starts_with(super::share::HEADER));
    assert!(!text.contains(&id(30).to_string()), "no folder id leaves");
    assert!(text.contains("# not shared"));
    let found = import(&text);
    assert!(found.recognised);
    assert_eq!(found.skipped, 0);
    assert_eq!(found.rules.len(), 2);
    assert!(found.rules[1].pre_trash && found.rules[1].skip_sorted);
    assert_eq!(found.hints, prefs.store_hints);
    assert_eq!(found.base.as_deref(), Some("Tidy"));
    assert_eq!(found.kinds.as_deref(), Some(&[Kind::Objects, Kind::Clothing][..]));
    // into someone else's settings: everything new, nothing private
    let mut theirs = Preferences::default();
    let mut pick = Pick::new(&found, &theirs);
    // a Pre-Trash rule and a new sort folder name wait for the user's tick
    assert_eq!(pick.rules, vec![true, false]);
    assert!(!pick.base && pick.stores && pick.kinds);
    pick.rules[1] = true;
    pick.base = true;
    found.apply(&mut theirs, &pick, false);
    assert_eq!(theirs.rules.len(), 2);
    assert!(theirs.excluded.is_empty() && theirs.rules.iter().all(|r| r.target.is_none()));
    assert!(theirs.stores && theirs.base == "Tidy");
    // a second import finds them all already there
    let pick = Pick::new(&found, &theirs);
    assert!(pick.rules.iter().all(|t| !t) && pick.hints.iter().all(|t| !t));
}

#[test]
fn shared_settings_skip_broken_lines_and_respect_the_ticks() {
    use super::share::{Pick, Share, export, import};
    let text = "# Aurora Auto-sort v1\nrule: any | chair sofa | Home\nrule: sometimes | x | y\nrule: all |  | Home\nstore: Nope = not-a-category\nstore: Birch Hollow = furniture\noption: kinds = objects, flying-cars\nhello there\n";
    let found = import(text);
    assert_eq!(found.rules.len(), 1);
    assert_eq!(found.hints.len(), 1);
    assert_eq!(found.skipped, 5);
    assert!(found.kinds.is_none());
    // only the ticked items go in
    let mut mine = Preferences::default();
    let pick = Pick {
        rules: vec![false],
        hints: vec![true],
        ..Default::default()
    };
    found.apply(&mut mine, &pick, false);
    assert!(mine.rules.is_empty() && mine.store_hints.len() == 1);
    // Replace clears my rules and hints first; Add keeps them
    let mut mine = Preferences {
        rules: vec![Rule {
            words: "lamp".into(),
            folder: "Lights".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let all = Pick::new(&found, &mine);
    let mut added = mine.clone();
    found.apply(&mut added, &all, false);
    assert_eq!(added.rules.len(), 2);
    found.apply(&mut mine, &all, true);
    assert_eq!(mine.rules.len(), 1);
    assert_eq!(mine.rules[0].words, "chair sofa");
    // unticked rules stay out of the shared text
    let share = Share {
        rules: vec![false, true],
        ..Share::all(&added)
    };
    let text = export(&added, &share);
    assert!(!text.contains("lamp") && text.contains("chair sofa"));
    // an oversized paste is cut, not refused
    let huge = "rule: any | chair | Home\n".repeat(10_000);
    let found = import(&huge);
    assert!(found.rules.len() <= super::share::MAX_RULES);
}

/// My Inventory with every default folder Aurora must never read inside
/// (each holding an old "denim freebie" and a delivery folder), the viewer
/// folders, Objects, and a folder of the user's.
fn hands_off_fixture() -> (Inventory, Vec<Uuid>) {
    let mut inv = fixture(&[]);
    let mut guarded = vec![id(11)];
    add_item(&mut inv, 1100, 11, "Old denim freebie in the trash", OLD);
    // every default folder type but Objects (6), and the old Outbox
    let kinds = [0, 1, 2, 3, 5, 7, 10, 13, 15, 16, 20, 21, 23, 46, 48, 50, 51, 53, 56, 57];
    for (n, kind) in kinds.iter().enumerate() {
        let folder_id = 2000 + n as u128 * 10;
        inv.folders
            .insert(id(folder_id), folder(id(folder_id), id(1), &format!("System {kind}"), *kind));
        inv.folders.get_mut(&id(1)).expect("root").children.push(id(folder_id));
        add_item(&mut inv, folder_id + 1, folder_id, "Old denim freebie", OLD);
        add_folder(&mut inv, folder_id + 2, folder_id, "Lumen - Denim Skirt - FATPACK");
        add_item(&mut inv, folder_id + 3, folder_id + 2, "Old denim skirt", OLD);
        add_folder(&mut inv, folder_id + 4, folder_id, "Empty inside");
        guarded.push(id(folder_id));
    }
    // an outfit inside My Outfits, and stock / version folders in Listings
    let outfits = 2000 + kinds.iter().position(|k| *k == 48).expect("my outfits") as u128 * 10;
    inv.folders.insert(id(2900), folder(id(2900), id(outfits), "Party outfit", 47));
    inv.folders.get_mut(&id(outfits)).expect("my outfits").children.push(id(2900));
    add_item(&mut inv, 2901, 2900, "Old denim jacket", OLD);
    let listings = 2000 + kinds.iter().position(|k| *k == 53).expect("listings") as u128 * 10;
    for (n, kind) in [54, 55].into_iter().enumerate() {
        let folder_id = 2910 + n as u128 * 10;
        inv.folders
            .insert(id(folder_id), folder(id(folder_id), id(listings), "Stock", kind));
        inv.folders.get_mut(&id(listings)).expect("listings").children.push(id(folder_id));
        add_item(&mut inv, folder_id + 1, folder_id, "Old denim freebie listed", OLD);
    }
    // the viewer folders: RLV shared folders, the AO, the bridge
    for (n, (name, inside)) in [
        ("#RLV", "Old denim freebie dress"),
        ("#Firestorm", "#AO"),
        ("#Aurora", "Aurora AO"),
        ("Animation Overrides", "Old denim freebie walks"),
    ]
    .into_iter()
    .enumerate()
    {
        let folder_id = 3000 + n as u128 * 10;
        add_folder(&mut inv, folder_id, 1, name);
        add_folder(&mut inv, folder_id + 1, folder_id, inside);
        add_item(&mut inv, folder_id + 2, folder_id + 1, "Old denim freebie dance", OLD);
        add_item(&mut inv, folder_id + 3, folder_id, "Old denim freebie bridge", OLD);
        add_folder(&mut inv, folder_id + 4, folder_id + 1, "Empty");
        guarded.push(id(folder_id));
    }
    // sortable: Objects' contents, a folder of the user's, and a `#` folder
    // that is not a viewer folder
    add_folder(&mut inv, 4020, 1, "#Hair");
    add_folder(&mut inv, 4021, 4020, "Bangs");
    add_item(&mut inv, 4022, 4020, "Old denim freebie hair", OLD);
    add_item(&mut inv, 4000, OBJECTS, "Old denim freebie skirt", OLD);
    add_folder(&mut inv, 4010, 1, "My clothes");
    add_folder(&mut inv, 4011, 4010, "Tops");
    add_item(&mut inv, 4012, 4010, "Old denim freebie top", OLD);
    inv.generation += 1;
    (inv, guarded)
}

/// Every rule, option and imported setting aimed at the hands-off folders.
fn aimed_at(guarded: &[Uuid]) -> Preferences {
    let mut rules = vec![
        Rule {
            words: "denim freebie old".into(),
            mode: Match::Any,
            pre_trash: true,
            ..Default::default()
        },
        Rule {
            words: "freebie".into(),
            mode: Match::Any,
            folder: "Trash/Freebies".into(),
            ..Default::default()
        },
    ];
    rules.extend(guarded.iter().map(|g| Rule {
        words: "denim".into(),
        target: Some(*g),
        ..Default::default()
    }));
    // what an imported text could set: a viewer folder as the sort folder
    let imported = super::share::import("# Aurora Auto-sort v1\noption: base = #RLV\noption: folders = on\n");
    let mut prefs = Preferences {
        rules,
        stores: true,
        ..Default::default()
    };
    imported.apply(
        &mut prefs,
        &super::share::Pick {
            stores: true,
            folders: true,
            kinds: true,
            base: true,
            ..Default::default()
        },
        false,
    );
    prefs
}

#[test]
fn hands_off_folders_are_never_read_moved_or_trashed_whatever_the_settings() {
    let (inv, guarded) = hands_off_fixture();
    let prefs = aimed_at(&guarded);
    assert_ne!(prefs.base_name(), "#RLV", "a viewer folder can never be the sort folder");
    let inside = |id: Uuid| guarded.iter().any(|g| actions::under(&inv, id, *g));
    let none = HashSet::new();
    // every way in: the whole inventory, each guarded folder as a source,
    // its entries chosen by hand, Pre-Trash rules, and Clean up
    let mut plans = vec![plan_whole(&inv, &prefs, &none, &none)];
    for g in &guarded {
        assert!(!source_allowed(&inv, *g), "{g} is not a source");
        assert!(!target_allowed(&inv, *g), "{g} is not a destination");
        assert!(hands_off(&inv, *g), "{g} never moves");
        plans.push(plan(&inv, *g, &prefs, &none, &none));
        let entries: Vec<Uuid> = inv.folders[g].items.iter().chain(&inv.folders[g].children).copied().collect();
        plans.push(plan_entries(&inv, &entries, &prefs, &none, &none));
    }
    for plan in &plans {
        for m in &plan.moves {
            assert!(!inside(m.id), "{} planned out of a hands-off folder", m.name);
            if let Dest::Folder(target) = m.to {
                assert!(!inside(target), "{} planned into a hands-off folder", m.name);
            }
        }
    }
    // the sortable places still sort
    let whole = &plans[0];
    assert!(whole.moves.iter().any(|m| m.id == id(4000)), "Objects' contents sort");
    assert!(whole.moves.iter().any(|m| m.id == id(4012)), "the user's own folder sorts");
    // ("#Hair" reads as a product: it moves whole into the sort folder)
    assert!(
        whole.moves.iter().any(|m| m.id == id(4020) || m.id == id(4022)),
        "an ordinary # folder sorts"
    );
    // the preview names the hands-off folders it leaves alone
    assert!(whole.kept_list.iter().any(|(name, why)| name == "#RLV" && *why == Kept::Protected));
    // Clean up, every option on
    for whole in [true, false] {
        let cleanup = Cleanup {
            whole,
            include_sorted: true,
            empty_folders: true,
            ..before_2020()
        };
        for source in std::iter::once(id(1)).chain(guarded.iter().copied()) {
            let rows = plan_cleanup(&inv, &prefs, &cleanup, source, today(), &none, &none);
            for row in &rows.rows {
                assert!(!inside(row.id) && !guarded.contains(&row.id), "{} cleaned up", row.name);
            }
        }
    }
    // and the last check before the Trash
    let holds_worn = HashSet::new();
    for (item_id, it) in inv.items.iter() {
        if inside(*item_id) {
            assert!(
                !trash_ready(&inv, *item_id, it.parent, &prefs, &none, &holds_worn),
                "{} trashed",
                it.name
            );
        }
    }
}

#[test]
fn folder_names_are_kept_in_ascii_so_runs_find_their_folders() {
    assert_eq!(inventory_name("Trié", "Sorted"), "Trie");
    assert_eq!(inventory_name("Pré-corbeille", "Pre-Trash"), "Pre-corbeille");
    assert_eq!(inventory_name("Café Noir", ""), "Cafe Noir");
    assert_eq!(inventory_name("整理済み", "Sorted"), "Sorted");
    assert_eq!(inventory_name("   ", "Sorted"), "Sorted");
    assert_eq!(inventory_name(&"x".repeat(80), "Sorted").len(), 63);
    // every category folder is ASCII whatever the language's name
    for category in Category::ALL {
        let name = category_folder(category);
        assert!(name.is_ascii() && !name.is_empty(), "{name:?}");
    }
    // a sort folder name that reaches somewhere it must not falls back
    for refused in ["#RLV", "Pre-Trash", "pre-corbeille", "Clothing", "Lost and Found", "My Outfits", ""] {
        let prefs = Preferences {
            base: refused.into(),
            ..Default::default()
        };
        assert!(!base_refused(&prefs.base_name()), "{refused}");
    }
}

#[test]
fn a_delivery_folder_whose_name_says_nothing_is_never_opened() {
    let mut inv = sorted_fixture();
    add_folder(&mut inv, 60, OBJECTS, "[Store] Aria");
    add_item(&mut inv, 600, 60, "Aria HUD", OLD);
    add_item(&mut inv, 601, 60, "Aria", NEW);
    let rows = cleaned(&plan_cleanup(
        &inv,
        &Preferences::default(),
        &before_2020(),
        id(1),
        today(),
        &HashSet::new(),
        &HashSet::new(),
    ));
    assert!(!rows.contains(&id(600)) && !rows.contains(&id(60)), "a mixed delivery stays whole");
    let plan = plan(&inv, id(OBJECTS), &word_rule(false), &HashSet::new(), &HashSet::new());
    assert!(!plan.moves.iter().any(|m| m.id == id(600)));
}

/// A tester's report: a folder of 4,000+ animations failed to sort. They
/// go now in batches of at most 200, every one of them, none refused.
#[test]
fn four_thousand_animations_sort_in_batches_with_no_failure() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 70, 1, "My dances");
    add_folder(&mut inv, 71, 70, "Favourites");
    inv.add_items(
        (0..4200u128)
            .map(|n| InvItem {
                id: id(100_000 + n),
                parent: id(70),
                name: format!("Dance {n}"),
                asset_type: 20,
                inv_type: 19,
                owner: id(AGENT),
                created_at: NEW,
                ..Default::default()
            })
            .collect(),
    );
    inv.generation += 1;
    let prefs = Preferences::default();
    let planned = plan(&inv, id(70), &prefs, &HashSet::new(), &HashSet::new());
    assert_eq!(planned.moves.len(), 4200);
    let mut queue: VecDeque<Move> = planned.moves.into();
    let mut skipped = 0;
    let mut batches = 0;
    while let Step::Change(change, moves) = step(
        &mut inv,
        &prefs.base_name(),
        &mut queue,
        &HashSet::new(),
        &HashSet::new(),
        &mut skipped,
    ) {
        assert!(moves.len() <= BATCH);
        assert!(change.operations.len() <= 2048, "far under the network layer's limit");
        actions::demo_mutate(&mut inv, id(AGENT), *change).expect("demo server");
        batches += 1;
    }
    assert_eq!(skipped, 0);
    assert_eq!(batches, 4200 / BATCH);
    assert!(inv.folders[&id(70)].items.is_empty(), "every animation moved");
    // the same animations in the Animations default folder stay: it is
    // hands-off, and the preview says why instead of failing
    let mut inv = fixture(&[]);
    inv.folders.insert(id(80), folder(id(80), id(1), "Animations", 20));
    inv.folders.get_mut(&id(1)).expect("root").children.push(id(80));
    add_typed(&mut inv, 800, 80, "Dance", NEW, 20);
    assert!(!source_allowed(&inv, id(80)));
    let planned = plan(&inv, id(80), &prefs, &HashSet::new(), &HashSet::new());
    assert!(planned.moves.is_empty());
    assert_eq!(planned.kept_list.first().map(|(_, why)| *why), Some(Kept::Protected));
}

/// A default folder the user allowed: its contents sort like their own
/// folders, the folder itself never moves, worn items and links stay; the
/// never-unlockable folders stay closed whatever is in the list; the
/// unlocks are never shared.
#[test]
fn an_allowed_default_folder_sorts_and_the_critical_ones_never_open() {
    let (mut inv, guarded) = hands_off_fixture();
    let clothing = guarded
        .iter()
        .copied()
        .find(|g| inv.folders[g].info.type_default == 5)
        .expect("clothing");
    let lost = guarded
        .iter()
        .copied()
        .find(|g| inv.folders[g].info.type_default == 16)
        .expect("lost and found");
    let none = HashSet::new();
    let prefs = Preferences {
        unlocked: guarded.clone(),
        include_lost_found: true,
        rules: vec![Rule {
            words: "denim".into(),
            folder: "Denim".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    inv.sorter_unlocked = prefs.unlocked_set(&inv);
    assert!(inv.sorter_unlocked.contains(&clothing) && inv.sorter_unlocked.contains(&lost));
    for g in &guarded {
        let kind = inv.folders[g].info.type_default;
        let allowed = unlockable(kind) || kind == 16;
        assert_eq!(source_allowed(&inv, *g), allowed, "type {kind}");
        assert!(hands_off(&inv, *g), "the default folder itself never moves");
    }
    // the critical ones stay closed even when listed
    for (kind, name) in [
        (14, "Trash"),
        (46, "Current outfit"),
        (48, "My outfits"),
        (53, "Listings"),
        (23, "Favorites"),
        (50, "Received"),
    ] {
        let found = guarded
            .iter()
            .any(|g| inv.folders.get(g).is_some_and(|f| f.info.type_default == kind) && folder_open(&inv, *g));
        assert!(!found, "{name} never opens");
    }
    let viewer = guarded.iter().copied().find(|g| inv.folders[g].info.name == "#RLV").expect("rlv");
    assert!(!folder_open(&inv, viewer));
    // Clothing's loose item sorts now; worn ones would not
    let planned = plan(&inv, clothing, &prefs, &none, &none);
    assert!(planned.moves.iter().any(|m| m.id == id(inv.folders[&clothing].items[0].as_u128())));
    let worn: HashSet<Uuid> = inv.folders[&clothing].items.iter().copied().collect();
    assert!(
        plan(&inv, clothing, &prefs, &none, &worn)
            .moves
            .iter()
            .all(|m| !worn.contains(&m.id))
    );
    // never shared
    let text = super::share::export(&prefs, &super::share::Share::all(&prefs));
    assert!(!text.contains(&clothing.to_string()) && !text.to_lowercase().contains("unlock") && !text.contains("lost"));
}

/// A large inventory: `items` items in Objects and in `folders` folders of
/// the user's (half of them deliveries), named like the demo's.
fn big_inventory(items: usize, folders: usize) -> Inventory {
    let mut inv = fixture(&[]);
    let names = [
        "Velvet skirt Reborn",
        "Lace bra Legacy",
        "Lumen - Hex Dress",
        "[Birch Hollow] + Oak table",
        "Mystery box",
        "Dance 3",
        "Old freebie",
    ];
    for n in 0..folders {
        let at = 1_000_000 + n as u128;
        let name = if n % 2 == 0 {
            format!("Store {n} - Gown FATPACK")
        } else {
            format!("My things {n}")
        };
        add_folder(&mut inv, at, if n % 2 == 0 { OBJECTS } else { 1 }, &name);
    }
    let mut batch = Vec::with_capacity(items);
    for n in 0..items {
        let parent = if n % 3 == 0 {
            id(OBJECTS)
        } else {
            id(1_000_000 + (n % folders.max(1)) as u128)
        };
        batch.push(InvItem {
            id: id(10_000_000 + n as u128),
            parent,
            name: format!("{} {n}", names[n % names.len()]),
            asset_type: 6,
            inv_type: 6,
            owner: id(AGENT),
            created_at: if n % 5 == 0 { OLD } else { NEW },
            ..Default::default()
        });
    }
    inv.add_items(batch);
    inv.generation += 1;
    inv
}

/// Timings at 400,000 items (run by hand: `cargo test --workspace
/// four_hundred_thousand -- --ignored --nocapture`).
#[test]
#[ignore = "a measure, about a minute in a debug build"]
fn four_hundred_thousand_items_timings() {
    let started = std::time::Instant::now();
    let inv = big_inventory(400_000, 40_000);
    eprintln!("build: {:?}", started.elapsed());
    let t = std::time::Instant::now();
    let copy = snapshot(&inv);
    eprintln!("snapshot: {:?} ({} items)", t.elapsed(), copy.items.len());
    drop(copy);
    let none = HashSet::new();
    let prefs = Preferences::default();
    let t = std::time::Instant::now();
    let whole = plan_whole(&inv, &prefs, &none, &none);
    eprintln!("plan_whole: {:?} ({} moves)", t.elapsed(), whole.moves.len());
    let t = std::time::Instant::now();
    let groups = whole.grouped().len();
    eprintln!("grouped: {:?} ({groups} groups)", t.elapsed());
    let t = std::time::Instant::now();
    let objects = plan(&inv, id(OBJECTS), &prefs, &none, &none);
    eprintln!("plan(Objects): {:?} ({} moves)", t.elapsed(), objects.moves.len());
    let t = std::time::Instant::now();
    let cleanup = plan_cleanup(&inv, &prefs, &before_2020(), id(1), today(), &none, &none);
    eprintln!("plan_cleanup: {:?} ({} rows)", t.elapsed(), cleanup.rows.len());
    let mut with_rule = prefs.clone();
    with_rule.rules.push(Rule {
        words: "freebie".into(),
        pre_trash: true,
        ..Default::default()
    });
    let t = std::time::Instant::now();
    let scan = pre_trash_scan(&inv, &with_rule, id(1), &none, &none);
    eprintln!("pre_trash_scan: {:?} ({} found)", t.elapsed(), scan.found.len());
    let mut inv = inv;
    let mut queue: VecDeque<Move> = whole.moves.into();
    let t = std::time::Instant::now();
    let mut batches = 0;
    while batches < 10
        && let Step::Change(_, _) = step(&mut inv, &prefs.base_name(), &mut queue, &none, &none, &mut 0)
    {
        batches += 1;
    }
    eprintln!("step x{batches}: {:?}", t.elapsed());
}

/// A guard against a slow path coming back (a lookup repeated per folder
/// made the whole-inventory plan of 400,000 items take 21 s instead of
/// 1 s): 50,000 items in 5,000 folders plan in well under the limit.
#[test]
fn a_large_inventory_plans_in_linear_time() {
    let inv = big_inventory(50_000, 5_000);
    let none = HashSet::new();
    let started = std::time::Instant::now();
    let whole = plan_whole(&inv, &Preferences::default(), &none, &none);
    let cleanup = plan_cleanup(&inv, &Preferences::default(), &before_2020(), id(1), today(), &none, &none);
    let elapsed = started.elapsed();
    assert!(!whole.moves.is_empty() && !cleanup.rows.is_empty());
    assert!(elapsed < std::time::Duration::from_secs(8), "{elapsed:?}");
}

#[test]
fn a_creator_hint_beats_the_store_read_from_the_name() {
    let maker = id(8701);
    let hints = [
        StoreHint {
            store: "Birch Hollow".into(),
            category: Category::Furniture,
            ..Default::default()
        },
        StoreHint {
            store: "Mossworks".into(),
            category: Category::Decor,
            creator: Some(maker),
            creator_name: "Made Up Maker".into(),
        },
    ];
    let by = |name: &str, creator: Option<Uuid>| match target_by(name, 6, 0, None, &[], true, &hints, creator) {
        Some(Dest::Path(path)) => path.join("/"),
        other => format!("{other:?}"),
    };
    // the creator's store wins over the tag in the name
    assert_eq!(
        by("[Birch Hollow] Lantern", Some(maker)),
        format!("{}/Mossworks", cat(Category::Decor))
    );
    // a name with its own category keeps it, under the creator's store
    assert_eq!(by("Velvet Sofa", Some(maker)), format!("{}/Mossworks", cat(Category::Furniture)));
    // someone else's item keeps the name's store
    assert_eq!(
        by("[Birch Hollow] Lantern", Some(id(8702))),
        format!("{}/Birch Hollow", cat(Category::Furniture))
    );
    // a creator hint without a store name only gives the category
    let bare = [StoreHint {
        category: Category::Decor,
        creator: Some(maker),
        ..Default::default()
    }];
    assert_eq!(
        match target_by("Thing 01", 6, 0, None, &[], true, &bare, Some(maker)) {
            Some(Dest::Path(path)) => path.join("/"),
            other => format!("{other:?}"),
        },
        cat(Category::Decor)
    );
}

#[test]
fn a_folder_takes_the_creator_of_most_of_its_items() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 40, OBJECTS, "Lantern Set FATPACK");
    for (n, maker) in [8701u128, 8701, 8702].into_iter().enumerate() {
        add_item(&mut inv, 400 + n as u128, 40, "Lantern", 0);
        inv.items.get_mut(&id(400 + n as u128)).expect("item").creator = id(maker);
    }
    assert_eq!(creator_of(&inv, id(40)), Some(id(8701)));
    assert_eq!(creator_of(&inv, id(402)), Some(id(8702)));
    // nobody known: no hint
    add_folder(&mut inv, 41, OBJECTS, "Empty box");
    assert_eq!(creator_of(&inv, id(41)), None);
    assert!(creator_hint(&inv, id(41), "x").is_none());
    let hint = creator_hint(&inv, id(40), "Made Up Maker").expect("hint");
    assert_eq!(hint.creator, Some(id(8701)));
    assert_eq!(hint.creator_name, "Made Up Maker");
    // the plan sorts the folder whole under the creator's store
    let prefs = Preferences {
        stores: true,
        folders: true,
        store_hints: vec![StoreHint {
            store: "Mossworks".into(),
            category: Category::Decor,
            creator: Some(id(8701)),
            creator_name: String::new(),
        }],
        ..Default::default()
    };
    let plan = plan(&inv, id(OBJECTS), &prefs, &HashSet::new(), &HashSet::new());
    let moved = plan.moves.iter().find(|m| m.id == id(40)).expect("the fatpack moves");
    assert!(path_of(&moved.to).ends_with("Mossworks"), "{:?}", moved.to);
}

#[test]
fn creator_hints_travel_in_shared_settings() {
    use super::share::{Share, export, import};
    let prefs = Preferences {
        store_hints: vec![StoreHint {
            store: "Mossworks".into(),
            category: Category::Decor,
            creator: Some(id(8701)),
            creator_name: "Made Up Maker".into(),
        }],
        ..Default::default()
    };
    let text = export(&prefs, &Share::all(&prefs));
    assert!(text.contains(&format!("creator: {}", id(8701))));
    let found = import(&text);
    assert_eq!(found.hints, prefs.store_hints);
    // a creator hint and a store hint of the same name are different hints
    let mine = Preferences {
        store_hints: vec![StoreHint {
            store: "Mossworks".into(),
            category: Category::Decor,
            ..Default::default()
        }],
        ..Default::default()
    };
    assert_eq!(found.duplicates(&mine).1, vec![false]);
    assert_eq!(found.duplicates(&prefs).1, vec![true]);
    // broken creator lines are skipped
    let bad = import("creator: not-an-id = A | decor\ncreator: 00000000-0000-0000-0000-000000000000 = A | decor\n");
    assert!(bad.hints.is_empty() && bad.skipped == 2);
}

#[test]
fn import_lists_every_changed_default_and_ticks_none_of_the_risky_ones() {
    use super::share::{OptionKind, Pick, import};
    let mine = Preferences {
        base: "Sorted".into(),
        stores: true,
        ..Default::default()
    };
    let text = format!(
        "{}\noption: base = Tidy\noption: stores = on\noption: folders = {}\nrule: any | old | Pre-Trash\nrule: any | chair | Home\n",
        super::share::HEADER,
        if mine.folders { "off" } else { "on" },
    );
    let found = import(&text);
    let changes = found.changes(&mine);
    // the same value is no change
    assert!(!changes.iter().any(|(k, _, _)| *k == OptionKind::Stores));
    let base = changes.iter().find(|(k, _, _)| *k == OptionKind::Base).expect("base change");
    assert_eq!((base.1.as_str(), base.2.as_str()), ("Sorted", "Tidy"));
    assert!(changes.iter().any(|(k, _, _)| *k == OptionKind::Folders));
    // base + folders + one Pre-Trash rule
    assert_eq!(found.defaults_changed(&mine), 3);
    let pick = Pick::new(&found, &mine);
    assert!(!pick.base, "a sort folder rename is never pre-ticked");
    assert_eq!(pick.rules, vec![false, true], "a Pre-Trash rule is never pre-ticked");
    // applying the default pick keeps the sort folder and takes no Pre-Trash rule
    let mut after = mine.clone();
    found.apply(&mut after, &pick, false);
    assert_eq!(after.base, "Sorted");
    assert!(after.rules.iter().all(|r| !r.pre_trash) && after.rules.len() == 1);
    assert_eq!(after.folders, !mine.folders);
    // unticked options stay
    let mut kept = mine.clone();
    found.apply(
        &mut kept,
        &Pick {
            rules: vec![false, false],
            ..Default::default()
        },
        false,
    );
    assert_eq!(kept, mine);
    // a Pre-Trash rule the user already has changes nothing
    let mut has_it = mine.clone();
    has_it.rules.push(found.rules[0].clone());
    assert_eq!(found.defaults_changed(&has_it), 2);
}

fn move_of(inv: &Inventory, at: u128, to: Dest) -> Move {
    Move {
        id: id(at),
        folder: inv.folders.contains_key(&id(at)),
        name: String::new(),
        from: actions::parent(inv, id(at)).expect("a parent"),
        to,
        label: String::new(),
        unpacker: false,
        restore: false,
    }
}

/// The sort folder itself, sent to a path below it that does not exist
/// yet, would go into a folder made inside itself: never sent.
#[test]
fn a_folder_never_goes_into_a_folder_made_inside_itself() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 50, 1, "Sorted");
    let mut queue: VecDeque<Move> = [move_of(&inv, 50, Dest::Path(vec!["Furniture".into()]))].into();
    let none = HashSet::new();
    let mut skipped = 0;
    let step = step(&mut inv, "Sorted", &mut queue, &none, &none, &mut skipped);
    assert!(matches!(step, Step::Done), "nothing sent");
    assert_eq!(skipped, 1);
}

/// A run read back from disk with a refused sort folder name (a viewer
/// folder) sends nothing into it; a path through Pre-Trash is refused too.
#[test]
fn a_resumed_run_never_files_into_a_refused_or_closed_folder() {
    let mut inv = fixture(&["Velvet Sofa", "Silk Lamp"]);
    add_folder(&mut inv, 3000, 1, "#RLV");
    let none = HashSet::new();
    let mut queue: VecDeque<Move> = [move_of(&inv, 100, Dest::Path(vec!["Furniture".into()]))].into();
    let mut skipped = 0;
    assert!(matches!(step(&mut inv, "#RLV", &mut queue, &none, &none, &mut skipped), Step::Done));
    assert_eq!(skipped, 1);
    // a rule's own folder inside Pre-Trash is no destination
    add_folder(&mut inv, 60, 1, &pre_trash_name());
    add_folder(&mut inv, 61, 60, "Keep maybe");
    assert!(!target_allowed(&inv, id(61)));
    assert!(!target_allowed(&inv, id(60)));
    // but Undo may put an entry back into Pre-Trash itself
    assert!(restore_target_allowed(&inv, id(60)));
    assert!(!restore_target_allowed(&inv, id(3000)));
    let mut queue: VecDeque<Move> = [move_of(&inv, 101, Dest::Folder(id(61)))].into();
    let mut skipped = 0;
    assert!(matches!(
        step(&mut inv, "Sorted", &mut queue, &none, &none, &mut skipped),
        Step::Done
    ));
    assert_eq!(skipped, 1);
}

/// A folder of the user's that holds an outfit, a typed folder or an
/// excluded folder never moves or goes to the Trash whole.
#[test]
fn a_folder_holding_an_outfit_or_an_excluded_folder_stays() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 70, OBJECTS, "Old stuff - FATPACK");
    add_item(&mut inv, 71, 70, "Old dress", OLD);
    inv.folders.insert(id(72), folder(id(72), id(70), "Party outfit", 47));
    inv.folders.get_mut(&id(70)).expect("old stuff").children.push(id(72));
    add_folder(&mut inv, 80, OBJECTS, "Older stuff - FATPACK");
    add_item(&mut inv, 81, 80, "Old dress", OLD);
    add_folder(&mut inv, 82, 80, "Keep this");
    add_item(&mut inv, 83, 82, "Old shoes", OLD);
    let none = HashSet::new();
    let prefs = Preferences {
        folders: true,
        excluded: vec![id(82)],
        ..Default::default()
    };
    let plan = plan_entries(&inv, &[id(70), id(80)], &prefs, &none, &none);
    assert!(plan.moves.iter().all(|m| m.id != id(70)), "the outfit's folder stays");
    let holds = actions::worn_ancestors(&inv, &none);
    assert!(!trash_ready(&inv, id(70), id(OBJECTS), &prefs, &none, &holds));
    assert!(!trash_ready(&inv, id(80), id(OBJECTS), &prefs, &none, &holds));
    // Clean up never takes them whole either
    let cleanup = plan_cleanup(&inv, &prefs, &before_2020(), id(1), today(), &none, &none);
    assert!(cleanup.rows.iter().all(|r| r.id != id(70) && r.id != id(80) && r.id != id(72)));
}

/// Replace keeps what the file holds too (shown "already in yours") and
/// the rules to the user's own folders, which no file can carry.
#[test]
fn replace_keeps_what_the_file_holds_too_and_my_own_folders() {
    use super::share::{Pick, import};
    let found = import(&format!(
        "{}\nrule: any | chair sofa | Home\nrule: any | lamp | Lights\n",
        super::share::HEADER
    ));
    let mut mine = Preferences {
        rules: vec![
            Rule {
                words: "chair sofa".into(),
                mode: Match::Any,
                folder: "Home".into(),
                ..Default::default()
            },
            Rule {
                words: "secret".into(),
                target: Some(id(30)),
                ..Default::default()
            },
            Rule {
                words: "old thing".into(),
                folder: "Attic".into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let pick = Pick::new(&found, &mine);
    assert_eq!(pick.rules, vec![false, true], "the duplicate is shown, not ticked");
    found.apply(&mut mine, &pick, true);
    let words: Vec<&str> = mine.rules.iter().map(|r| r.words.as_str()).collect();
    assert_eq!(words, vec!["chair sofa", "secret", "lamp"]);
}

/// An object whose contents have several creators has none ("unknown /
/// multiple"): the account it came from (the store's) stands in, never the
/// owner; a folder of such objects takes the store too.
#[test]
fn an_object_with_unknown_creators_is_known_by_the_store_it_came_from() {
    let mut inv = fixture(&[]);
    let store = id(8705);
    add_folder(&mut inv, 90, OBJECTS, "Ryu Jeans - FATPACK");
    for n in 0..3u128 {
        add_item(&mut inv, 91 + n, 90, "Ryu Jeans", 0);
        let it = inv.items.get_mut(&id(91 + n)).expect("item");
        it.creator = Uuid::nil();
        it.last_owner = store;
    }
    assert_eq!(creator_of(&inv, id(91)), Some(store));
    assert_eq!(creator_of(&inv, id(90)), Some(store));
    // an item the owner rezzed and took back: last owner is the owner
    add_item(&mut inv, 95, OBJECTS, "My build", 0);
    let it = inv.items.get_mut(&id(95)).expect("item");
    it.creator = Uuid::nil();
    it.last_owner = it.owner;
    assert_eq!(creator_of(&inv, id(95)), None);
    // a creator hint for the store sorts the fatpack under it
    let prefs = Preferences {
        stores: true,
        folders: true,
        store_hints: vec![StoreHint {
            store: "Ryu Store".into(),
            category: Category::Pants,
            creator: Some(store),
            creator_name: String::new(),
        }],
        ..Default::default()
    };
    let plan = plan(&inv, id(OBJECTS), &prefs, &HashSet::new(), &HashSet::new());
    let moved = plan.moves.iter().find(|m| m.id == id(90)).expect("the fatpack moves");
    assert!(path_of(&moved.to).ends_with("Ryu Store"), "{:?}", moved.to);
}

/// Lost and Found opens by its own switch only: listed among the unlocked
/// folders (an edited setting) it stays closed.
#[test]
fn lost_and_found_opens_by_its_switch_only() {
    let (mut inv, guarded) = hands_off_fixture();
    let lost = guarded
        .iter()
        .copied()
        .find(|g| inv.folders[g].info.type_default == 16)
        .expect("lost and found");
    let listed = Preferences {
        unlocked: vec![lost],
        ..Default::default()
    };
    inv.sorter_unlocked = listed.unlocked_set(&inv);
    assert!(!folder_open(&inv, lost));
    let switched = Preferences {
        include_lost_found: true,
        ..Default::default()
    };
    inv.sorter_unlocked = switched.unlocked_set(&inv);
    assert!(folder_open(&inv, lost) && source_allowed(&inv, lost));
    assert!(hands_off(&inv, lost), "the folder itself never moves");
}
