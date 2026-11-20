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
    // (each fixture its own generation: the folder judgement's memo is
    // keyed by it, and a test's layouts are built in the same place)
    static FIXTURES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let mut inv = Inventory {
        root: id(1),
        generation: FIXTURES.fetch_add(1 << 32, std::sync::atomic::Ordering::Relaxed),
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
        ("VELA.mods - Soft Thighs V4.2 - eBody Reborn", "Reborn/Body add-ons"),
        (
            ". i i c i n g . // duo toned stockings - REBORN (add me)",
            "Reborn/Stockings & socks",
        ),
        ("(Nuit Pale*) Reborn+Luma Stockings - Lamb", "Reborn/Stockings & socks"),
        ("KORU TATTOO - Mother of Dragons - BoM/EVO-X [FATPACK] [REBORN]", "Reborn/Tattoos"),
        ("18.Brisk - Rope Play GACHA - Black Ribbon Garter - Reborn", "Reborn/Lingerie"),
        ("2. Tolva - Alyssa Lingerie Set Reborn Red RARE", "Reborn/Lingerie"),
        ("4. Tolva Cosette Corset Reborn+Juicy Black", "Reborn/Lingerie"),
        ("18. Tolva Cosette Robe Reborn+Juicy Black", "Reborn/Robes & sleepwear"),
        ("3. K-Duo - Kristina Shirt // Reborn", "Reborn/Tops"),
        ("REBORN - Waifu Boobs - Wear to unpack", "Reborn/Body add-ons"),
        ("Velvet Ruin - Maia - Reborn (Black)", "Reborn"),
        (".QW. Angel Set - Reborn Pack [Unpacker]", "Reborn/Outfits & sets"),
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
    assert_eq!(classify("Juicy Larkspur top - Legacy", 6, 0).body, Some(Body::Named("Legacy")));
    // whole words only: "Unreborn" or "stop" name nothing
    assert_eq!(classify("Unreborn stop", 6, 0), Classified::default());
}

#[test]
fn unpackers_are_recognised_and_sorted_with_their_product() {
    for name in [
        "eBODY - REBORN - Wear to unpack",
        ".QW. Vixen Dress - Reborn Pack [Unpacker]",
        "zph - my body reflection - reborn version (packed)",
    ] {
        assert!(classify(name, 6, 0).unpacker, "{name}");
    }
    assert!(!classify("Packer jacket", 6, 0).unpacker);
    assert_eq!(path(".QW. Vixen Dress - Reborn Pack [Unpacker]").as_deref(), Some("Reborn/Dresses"));
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
        .insert(unpacked, folder(unpacked, id(OBJECTS), ".QW. Vixen Dress Reborn", -1));
    inv.folders.insert(id(201), folder(id(201), id(OBJECTS), "Reborn", -1));
    inv.folders.get_mut(&id(OBJECTS)).expect("objects").children = vec![unpacked, id(201)];
    let worn: HashSet<Uuid> = [id(101)].into();
    let plan = plan(&inv, id(OBJECTS), &Preferences::default(), &HashSet::new(), &worn);
    let moved: Vec<_> = plan.moves.iter().map(|m| (m.name.as_str(), path_of(&m.to))).collect();
    assert_eq!(
        moved,
        [
            (".QW. Vixen Dress Reborn", "Reborn/Dresses".to_owned()),
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
    assert_eq!(
        store("[Birch Hollow] + Bramble Round Gothic Table").as_deref(),
        Some("Birch Hollow")
    );
    assert_eq!(store("*Neverlace* Neon Heart Shelf").as_deref(), Some("Neverlace"));
    assert_eq!(store(".QZ. Vixen Dress").as_deref(), Some("QZ"));
    assert_eq!(store("Kitty - Hex Dress (Reborn Squish) - FATPACK").as_deref(), Some("Kitty"));
    assert_eq!(store("zph // Platform Boots").as_deref(), Some("zph"));
    // a body tag or a product before the dash is no store
    assert_eq!(store("[REBORN] Denim Skirt"), None);
    assert_eq!(store("Reborn - Waifu Boobs"), None);
    assert_eq!(store("Velvet Sofa Modular - PG"), None);
    assert_eq!(store("Mystery box"), None);
    let to = |name: &str| match target(name, 6, 0, None, &[], true) {
        Some(Dest::Path(path)) => path.join("/"),
        other => format!("{other:?}"),
    };
    assert_eq!(to("[Birch Hollow] + Orlen Pillow Couch (black)"), "Furniture/Birch Hollow");
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
    assert_eq!(whole_sources(&inv, &prefs), [id(1), id(OBJECTS), home]);
    let plan = plan_whole(&inv, &prefs, &HashSet::new(), &HashSet::new());
    let mut moved: Vec<_> = plan.moves.iter().map(|m| m.id).collect();
    moved.sort();
    // "My chairs", the user's shelf at the root, is opened even named for
    // furniture: its chair is sorted, the shelf stays
    assert_eq!(moved, [fatpack, id(100), id(150), id(151)]);
    // excluding Objects leaves the root's loose item and its delivery
    let prefs = Preferences {
        excluded: vec![id(OBJECTS)],
        ..Default::default()
    };
    let plan = plan_whole(&inv, &prefs, &HashSet::new(), &HashSet::new());
    let mut moved: Vec<_> = plan.moves.iter().map(|m| m.id).collect();
    moved.sort();
    assert_eq!(moved, [id(150), id(151)]);
}

const OLD: i64 = 1_400_000_000;
const NEW: i64 = 1_700_000_000;

fn add_folder(inv: &mut Inventory, at: u128, parent: u128, name: &str) {
    // (a change, as the inventory counts one: the folder judgement's memo
    // never answers for another layout built in the same place)
    inv.generation += 1;
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

/// A whole sort opens the user's own folders, even named for a category or
/// a body, and moves the products inside them whole.
#[test]
fn a_whole_sort_opens_the_users_folders_and_moves_products_whole() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 80, 1, "My Dresses");
    add_folder(&mut inv, 81, 80, "Velmora - Hex Dress");
    add_item(&mut inv, 200, 81, "Velmora - Hex Dress Maitreya", 0);
    add_folder(&mut inv, 82, 1, "Stuff");
    add_item(&mut inv, 201, 82, "Velvet Hair Bow", 0);
    add_folder(&mut inv, 83, 1, "Reborn Stuff");
    add_folder(&mut inv, 84, 83, "Winter");
    add_folder(&mut inv, 85, 84, "[Velmora] Coat");
    add_item(&mut inv, 202, 85, "Coat", 0);
    let none = HashSet::new();
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
    for own in [80, 82, 83, 84] {
        assert!(!moved.contains(&id(own)), "the user's folder {own} is opened, never moved");
    }
    assert!(moved.contains(&id(81)) && moved.contains(&id(85)), "products move whole");
    assert!(moved.contains(&id(201)), "a loose item of Stuff is sorted");
    assert!(!moved.contains(&id(200)) && !moved.contains(&id(202)), "a product is never opened");
    // a product named with a word some bodies share (Venus), and a store's
    // delivery whose name says no kind: never opened
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 86, 1, "Venus Dress");
    add_item(&mut inv, 207, 86, "Venus Dress Maitreya", 0);
    add_folder(&mut inv, 87, 1, "Velmora - Aphrodite");
    add_folder(&mut inv, 88, 87, "Aphrodite Maitreya");
    add_folder(&mut inv, 89, 87, "Aphrodite Legacy");
    add_item(&mut inv, 208, 87, "Aphrodite HUD", 0);
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
    assert!(moved.contains(&id(86)) && !moved.contains(&id(207)), "Venus Dress moves whole");
    assert!(
        ![88, 89, 208].iter().any(|n| moved.contains(&id(*n))),
        "the delivery is never opened"
    );
    // deliveries with their HUD, textures or fatpack folders: never opened
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 70, 1, "Velmora - Aphrodite");
    add_folder(&mut inv, 71, 70, "Aphrodite Maitreya");
    add_folder(&mut inv, 72, 70, "Aphrodite HUD");
    add_folder(&mut inv, 73, 1, "Velmora Hex Dress");
    add_folder(&mut inv, 74, 73, "HUD");
    add_item(&mut inv, 209, 73, "Hex Dress Maitreya", 0);
    add_folder(&mut inv, 75, 1, "Hex Gown");
    add_folder(&mut inv, 76, 75, "Maitreya");
    add_folder(&mut inv, 77, 75, "Fatpack");
    add_folder(&mut inv, 78, 1, "Clothes");
    add_folder(&mut inv, 79, 78, "Reborn");
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
    assert!(
        ![71, 72, 74, 76, 77, 209].iter().any(|n| moved.contains(&id(*n))),
        "deliveries never opened"
    );
    assert!(moved.contains(&id(73)) && moved.contains(&id(75)), "named deliveries move whole");
    assert!(!moved.contains(&id(78)), "Clothes is a shelf");
    // a skin by its head versions, a store's delivery with its HUD folder,
    // a shelf named for an occasion
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 60, 1, "Velmora - Ivy");
    add_folder(&mut inv, 61, 60, "Lelutka");
    add_folder(&mut inv, 62, 60, "Catwa");
    add_folder(&mut inv, 63, 1, "Velmora - Aphrodite");
    add_folder(&mut inv, 64, 63, "HUD");
    add_item(&mut inv, 210, 63, "Aphrodite Maitreya", 0);
    add_folder(&mut inv, 65, 1, "Party Dresses");
    add_item(&mut inv, 211, 65, "Silk Gown", 0);
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
    assert!(
        ![61, 62, 64, 210].iter().any(|n| moved.contains(&id(*n))),
        "deliveries never opened"
    );
    assert!(!moved.contains(&id(65)) && moved.contains(&id(211)), "Party Dresses is a shelf");
    // an occasion beside one thing named is a product, beside a plural a shelf
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 66, 1, "Christmas Sweater");
    add_item(&mut inv, 212, 66, "Christmas Sweater Maitreya", 0);
    add_folder(&mut inv, 69, 1, "Party Dress");
    add_item(&mut inv, 214, 69, "Party Dress Reborn", 0);
    add_folder(&mut inv, 67, 1, "Avatar");
    add_folder(&mut inv, 68, 67, "Lelutka");
    add_item(&mut inv, 213, 68, "Lelutka Head", 0);
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
    assert!(
        moved.contains(&id(66)) && !moved.contains(&id(212)),
        "Christmas Sweater moves whole"
    );
    assert!(!moved.contains(&id(67)), "Avatar is a shelf");
    // an occasion-named delivery with its versions, an outfit with its parts
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 50, 1, "Winter Boots");
    add_folder(&mut inv, 51, 50, "Maitreya");
    add_folder(&mut inv, 52, 50, "HUD");
    add_folder(&mut inv, 53, 1, "Hex Outfit");
    add_folder(&mut inv, 54, 53, "Top");
    add_folder(&mut inv, 55, 53, "Pants");
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
    assert!(![51, 52, 54, 55].iter().any(|n| moved.contains(&id(*n))), "never opened");
    assert!(moved.contains(&id(50)) && moved.contains(&id(53)), "they move whole");
    // a theme or season shelf sorted by kind is opened
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 40, 1, "Fantasy");
    add_folder(&mut inv, 41, 40, "Heads");
    add_folder(&mut inv, 42, 40, "Skins");
    add_folder(&mut inv, 43, 1, "Winter");
    add_folder(&mut inv, 44, 43, "Maitreya");
    add_folder(&mut inv, 45, 43, "Boots");
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
    assert!(!moved.contains(&id(40)) && !moved.contains(&id(43)), "shelves are opened");
    // deliveries with plural parts or fatpack folders stay whole
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 30, 1, "Hex Outfit");
    add_folder(&mut inv, 31, 30, "Top");
    add_folder(&mut inv, 32, 30, "Shoes");
    add_folder(&mut inv, 33, 1, "Kira Avatar");
    add_folder(&mut inv, 34, 33, "Head");
    add_folder(&mut inv, 35, 33, "Eyes");
    add_item(&mut inv, 2900, 34, "Kira Head", 0);
    add_folder(&mut inv, 36, 1, "Hex Dress");
    add_folder(&mut inv, 37, 36, "Maitreya");
    add_folder(&mut inv, 38, 36, "HUDs");
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
    assert!(
        ![31, 32, 34, 35, 37, 38, 2900].iter().any(|n| moved.contains(&id(*n))),
        "never opened"
    );
    // (Kira Avatar names no kind: it stays whole where it is)
    assert!([30, 36].iter().all(|n| moved.contains(&id(*n))), "they move whole");
    // a delivery with only its HUD and textures folders, and an occasion
    // shelf of a plural kind with body folders
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 20, 1, "Velmora Aphrodite");
    add_folder(&mut inv, 21, 20, "HUD");
    add_folder(&mut inv, 22, 20, "Textures");
    add_item(&mut inv, 215, 21, "Aphrodite HUD", 0);
    add_folder(&mut inv, 23, 1, "Party Dresses");
    add_folder(&mut inv, 24, 23, "Maitreya");
    add_folder(&mut inv, 25, 23, "Reborn");
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
    assert!(
        ![21, 22, 215].iter().any(|n| moved.contains(&id(*n))),
        "the delivery is never opened"
    );
    assert!(!moved.contains(&id(23)), "Party Dresses is a shelf");
    // a mesh body's, a furniture set's and a skin pack's own folders
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 10, 1, "Velmora Body Delivery");
    add_folder(&mut inv, 12, 10, "Hands");
    add_folder(&mut inv, 13, 10, "Feet");
    add_folder(&mut inv, 14, 1, "Boho Bedroom Set");
    add_folder(&mut inv, 15, 14, "PG");
    add_folder(&mut inv, 16, 14, "Adult");
    add_folder(&mut inv, 17, 1, "Lumen Skin - Lelutka");
    add_folder(&mut inv, 18, 17, "Light");
    add_folder(&mut inv, 19, 17, "Dark");
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
    assert!(
        ![12, 13, 15, 16, 18, 19].iter().any(|n| moved.contains(&id(*n))),
        "deliveries never opened"
    );
    // an AO, a car, a tattoo pack and a prefab with their working parts; a
    // builder's shelf of textures and scripts stays a shelf
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 120, 1, "Elegance AO");
    add_folder(&mut inv, 121, 120, "Stands");
    add_folder(&mut inv, 122, 120, "Walks");
    add_folder(&mut inv, 123, 1, "Speedster Car");
    add_folder(&mut inv, 124, 123, "Textures");
    add_folder(&mut inv, 125, 123, "Scripts");
    add_folder(&mut inv, 126, 1, "Rose Sleeve Tattoo");
    add_folder(&mut inv, 127, 126, "BOM");
    add_folder(&mut inv, 128, 126, "Opacity");
    add_folder(&mut inv, 129, 1, "Seaside Cottage Prefab");
    add_folder(&mut inv, 130, 129, "Rez Box");
    add_folder(&mut inv, 131, 1, "Builds");
    add_folder(&mut inv, 132, 131, "Textures");
    add_folder(&mut inv, 133, 131, "Scripts");
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
    assert!(
        ![121, 122, 124, 125, 127, 128, 130].iter().any(|n| moved.contains(&id(*n))),
        "deliveries never opened"
    );
    assert!(!moved.contains(&id(131)), "Builds is a shelf");
    // a named delivery with subfolders no list knows stays whole
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 140, 1, "Animesh Corgi Pet");
    add_folder(&mut inv, 141, 140, "Food");
    add_folder(&mut inv, 142, 140, "Manual");
    add_folder(&mut inv, 143, 1, "Desert Eagle Gun");
    add_folder(&mut inv, 144, 143, "Holster");
    add_folder(&mut inv, 145, 1, "Velmora");
    add_folder(&mut inv, 146, 145, "Velmora - Hex Dress");
    add_folder(&mut inv, 147, 145, "Velmora - Ivy Boots");
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
    assert!(![141, 142, 144].iter().any(|n| moved.contains(&id(*n))), "deliveries never opened");
    // (a store's shelf named only for the store stays whole in place: its
    // deliveries are never opened either)
    assert!(!moved.contains(&id(145)), "a store's shelf never moves");
    // a shelf named for a plural kind is opened (`Work Outfits`)
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 150, 1, "Work Outfits");
    add_folder(&mut inv, 151, 150, "Monday");
    add_item(&mut inv, 216, 151, "Silk Shirt", 0);
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
    assert!(!moved.contains(&id(150)), "Work Outfits is a shelf");
    // a shelf of items named for a kind or a body is opened too
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 93, 1, "My Shoes");
    add_item(&mut inv, 205, 93, "Lace Boots", 0);
    add_folder(&mut inv, 94, 1, "Hex Dress");
    add_item(&mut inv, 206, 94, "Hex Dress Maitreya", 0);
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
    assert!(!moved.contains(&id(93)) && moved.contains(&id(205)), "My Shoes is opened");
    assert!(moved.contains(&id(94)) && !moved.contains(&id(206)), "Hex Dress moves whole");
    // shelves named for a kind are opened, whatever their subfolders
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 95, 1, "Dresses");
    add_folder(&mut inv, 96, 95, "Reborn");
    add_folder(&mut inv, 97, 1, "Shoes");
    add_folder(&mut inv, 98, 97, "Shoes Reborn");
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
    assert!(!moved.contains(&id(95)) && !moved.contains(&id(97)), "shelves are opened");
    // a product of folders with no tag: its versions keep it whole
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 90, 1, "Hex Dress");
    add_folder(&mut inv, 91, 90, "Maitreya");
    add_folder(&mut inv, 92, 90, "Hex Dress Legacy");
    add_item(&mut inv, 203, 90, "Hex Dress HUD", 0);
    add_item(&mut inv, 204, 91, "Hex Dress Maitreya", 0);
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
    assert!(moved.contains(&id(90)), "the product moves whole");
    assert!(![91, 92, 203, 204].iter().any(|n| moved.contains(&id(*n))), "never opened");
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

/// A longer phrase built on the word a category matched wins (`hair bow`
/// is no hair), and a store is read from any of the ways it is written.
#[test]
fn phrases_beat_the_word_they_extend_and_stores_are_read_every_way() {
    use Category::*;
    let category = |name: &str| classify(name, 6, 0).category;
    assert_eq!(category("Velvet Hair Bow"), Some(Accessories));
    assert_eq!(category("Silk Dress Shirt"), Some(Tops));
    assert_eq!(category("Cat Eye Sunglasses"), Some(Accessories));
    assert_eq!(category("Wooden Shoe Rack"), Some(Furniture));
    assert_eq!(category("Teddy Bear"), Some(Decor));
    assert_eq!(category("Rustic Bedroom Set"), Some(Furniture));
    // another category's own words never override the first
    assert_eq!(category("Dragon Tattoo for Evo X"), Some(Tattoos));
    assert_eq!(category("Lace Dress"), Some(Dresses));
    assert_eq!(store("**Velmora** Hex Dress").as_deref(), Some("Velmora"));
    assert_eq!(store("..::North Oak::.. Table").as_deref(), Some("North Oak"));
    assert_eq!(store("(Mesh) Velmora - Hex Dress").as_deref(), Some("Velmora"));
    assert_eq!(store("[Moon Hair] Ada").as_deref(), Some("Moon Hair"));
    assert_eq!(store("[Hair] Ada"), None);
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
        ("Brindlewood // Bento Money Envelope (Packed)", -1, format!("{stores}/Brindlewood")),
        ("TUMBLINGPUPPET", -1, none.clone()),
        ("ZyLo 1.2.5 Box", -1, none.clone()),
        (
            "hey-you: Pip Candy Striped Thigh Socks  - Aniseed (Wear)",
            -1,
            format!("{}/hey-you", cat(Stockings)),
        ),
        ("StratusEdge Combat System V27.514", -1, none.clone()),
        ("{RW} Such a S****y Strike Hud [boxed]", -1, format!("{}/RW", cat(Huds))),
        ("~JJ~ The Lizard Glow Set", -1, format!("{}/JJ", cat(Outfits))),
        // objects
        (
            "[XQ][GESTURES][1] PRAIRIE PEACH TART PACK (ADD+CLICK)",
            6,
            format!("{}/XQ", cat(Gestures)),
        ),
        ("Velmora - Ryu Jeans - FATPACK", 6, format!("{}/Velmora", cat(Pants))),
        ("LeLUTKA Otto Head 4.0", 6, cat(Heads).to_owned()),
        ("MossPixel_Terminal_Office_High1", 6, format!("{stores}/MossPixel")),
        ("MossPx_Sign_F", 6, format!("{stores}/MossPx")),
        ("Object", 6, none.clone()),
        ("[Thornlace] Rebellion Jacket // Fatpack", 6, format!("{}/Thornlace", cat(Jackets))),
        ("Ashgrove Homes ~ Eclipse (ADD)", 6, format!("{stores}/Ashgrove Homes")),
        ("MossPixel Deposit HUD", 6, cat(Huds).to_owned()),
        ("Q_Scanner_Briny", 6, none.clone()),
        ("Nyla - Slow Unwind - Bom Bandages Fatpack", 6, format!("{}/Nyla", cat(Tattoos))),
        // a product's HUD goes with the product
        ("[Thornlace] Trauma Tee // Fatpack Hud", 6, format!("{}/Thornlace", cat(Tops))),
        // LegacyM: Legacy's male body; AT, its Athletic shape
        (
            "[Thornlace] Trauma Tee // Fatpack // LegacyM+AT",
            6,
            format!("Legacy Male/{}/Thornlace", cat(Tops)),
        ),
        (
            "QuillFox Willy Wrappers - Glove",
            6,
            format!("{}/QuillFox Willy Wrappers", cat(Gloves)),
        ),
        ("Garden Party Invitation", 6, none.clone()),
        ("Mouth Sprinkler (Light Pink)", 6, none.clone()),
        // Waifus, Reborn's body mod: the mod itself is an add-on, its clothes
        // get a Waifus folder (with Squish when both)
        ("Waifu REBORN - Waifus - Addon v2.0", 6, format!("Reborn/{}", cat(BodyAddons))),
        (
            "Reborn Feat. Marlowe - Waifu Bikini - Top Slim",
            6,
            format!("Reborn/{}/Waifus", cat(Swimwear)),
        ),
        (
            "Larkspur - Lovesick Darling - Reborn Waifus - Bra",
            6,
            format!("Reborn/{}/Larkspur/Waifus", cat(Lingerie)),
        ),
        (
            "Fenwick - Marisol Top - Black - [Waifus size Squish]",
            6,
            format!("Reborn/{}/Fenwick/Waifus Squish", cat(Tops)),
        ),
        // a body skin, named by its body and tone; a body mod maker
        (
            "THISTLE: The \"Harbor Body\" for eBody Reborn (Porcelain)",
            6,
            format!("Reborn/{}/THISTLE", cat(Skins)),
        ),
        (
            "VELA.mods - Soft Thighs V4.2 - eBody Reborn",
            6,
            format!("Reborn/{}/VELA.mods", cat(BodyAddons)),
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
        store: "ashgrove homes".into(),
        category: Category::Furniture,
        ..Default::default()
    }];
    assert_eq!(
        sorted_as("Ashgrove Homes ~ Eclipse (ADD)", 6, &hints),
        format!("{}/Ashgrove Homes", Category::Furniture.folder())
    );
    // a name's own category wins over the hint
    let hints = [StoreHint {
        store: "Velmora".into(),
        category: Category::Furniture,
        ..Default::default()
    }];
    assert_eq!(
        sorted_as("Velmora - Ryu Jeans - FATPACK", 6, &hints),
        format!("{}/Velmora", Category::Pants.folder())
    );
    // without a folder per store, the hint still files it
    assert_eq!(
        target_with(
            "Ashgrove Homes ~ Eclipse (ADD)",
            6,
            0,
            None,
            &[],
            false,
            &[StoreHint {
                store: "Ashgrove Homes".into(),
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
    assert!(
        text.contains("is not shared") && !text.contains("secret stash"),
        "a private rule's words stay private"
    );
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
    // unlocked as a source only: a rule never files anything into them
    assert!(!target_allowed(&inv, clothing) && !target_allowed(&inv, lost));
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
    let t = std::time::Instant::now();
    let layout = journal::Snapshot::take(&inv, 0, id(AGENT));
    eprintln!("layout snapshot: {:?} ({} items)", t.elapsed(), layout.items.len());
    drop(layout);
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
    let scan = scan_rules(&inv, &with_rule, id(1), &none, &none);
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
    let by = |name: &str, creator: Option<Uuid>| match target_by(name, 6, 0, None, &[], true, &hints, creator, None) {
        Some(Dest::Path(path)) => path.join("/"),
        other => format!("{other:?}"),
    };
    // the creator's store wins over the tag in the name
    assert_eq!(
        by("[Birch Hollow] Vesper", Some(maker)),
        format!("{}/Mossworks", cat(Category::Decor))
    );
    // a name with its own category keeps it, under the creator's store
    assert_eq!(by("Velvet Sofa", Some(maker)), format!("{}/Mossworks", cat(Category::Furniture)));
    // someone else's item keeps the name's store
    assert_eq!(
        by("[Birch Hollow] Vesper", Some(id(8702))),
        format!("{}/Birch Hollow", cat(Category::Furniture))
    );
    // a creator hint without a store name only gives the category
    let bare = [StoreHint {
        category: Category::Decor,
        creator: Some(maker),
        ..Default::default()
    }];
    assert_eq!(
        match target_by("Thing 01", 6, 0, None, &[], true, &bare, Some(maker), None) {
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
        from_trash: false,
        snapshot: false,
        only_if_empty: false,
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

/// Imported text can neither make a rule a hundred folders deep nor turn
/// every type off with an empty list, nor show a refused sort folder name
/// as a change.
#[test]
fn imported_paths_types_and_names_are_bounded() {
    use super::share::{OptionKind, import};
    let deep = "a/".repeat(100);
    let found = import(&format!(
        "{}\nrule: any | chair | {deep}\noption: kinds =\noption: base = #RLV\n",
        super::share::HEADER
    ));
    assert!(split_path(&found.rules[0].folder).len() <= 8);
    assert!(found.kinds.is_none(), "an empty list is not read");
    let mine = Preferences::default();
    assert!(!found.changes(&mine).iter().any(|(k, _, _)| *k == OptionKind::Base));
}

/// After a change of language, the sort folder and Pre-Trash made in the
/// old one are still Auto-sort's: found, reused, never sorted.
#[test]
fn the_sort_and_pre_trash_folders_are_found_in_another_language() {
    let mut inv = fixture(&["Velvet Sofa"]);
    let sorted_fr = inventory_name(aurora_i18n::text_in("fr", "inventory-sorter-default-base"), "Sorted");
    let pre_trash_fr = inventory_name(aurora_i18n::text_in("fr", "inventory-sorter-pre-trash"), "Pre-Trash");
    // the test runs in English: the French names differ
    assert_ne!(sorted_fr, Preferences::default().base_name());
    add_folder(&mut inv, 50, 1, &sorted_fr);
    add_folder(&mut inv, 60, 1, &pre_trash_fr);
    let prefs = Preferences::default();
    assert_eq!(base_folder(&inv, &prefs), Some(id(50)));
    assert_eq!(base_name_in(&inv, &prefs), sorted_fr);
    assert_eq!(pre_trash(&inv), Some(id(60)));
    assert!(!target_allowed(&inv, id(60)), "still Pre-Trash");
    // a whole sort never takes them as sources
    let whole = plan_whole(&inv, &prefs, &HashSet::new(), &HashSet::new());
    assert!(whole.moves.iter().all(|m| m.id != id(50) && m.id != id(60)));
    // a sort files into the French sort folder instead of making another
    let planned = plan(&inv, id(OBJECTS), &prefs, &HashSet::new(), &HashSet::new());
    let mut queue: VecDeque<Move> = planned.moves.into();
    let mut skipped = 0;
    let base = base_name_in(&inv, &prefs);
    let Step::Change(change, _) = step(&mut inv, &base, &mut queue, &HashSet::new(), &HashSet::new(), &mut skipped) else {
        panic!("a batch")
    };
    let made_at_root = change
        .operations
        .iter()
        .any(|op| matches!(op, Operation::Folder { parent, .. } if *parent == id(1)));
    assert!(!made_at_root, "no second sort folder at the root");
    // a name the user typed is only ever that name
    let named = Preferences {
        base: "Tidy".into(),
        ..Default::default()
    };
    assert_eq!(base_folder(&inv, &named), None);
}

/// After a change of language, Clean up and the Pre-Trash scan never take
/// the old-language sort folder (or an old Pre-Trash) whole, however old
/// all it holds is.
#[test]
fn clean_up_never_takes_the_sort_folder_whole_after_a_change_of_language() {
    let mut inv = fixture(&[]);
    let sorted_fr = inventory_name(aurora_i18n::text_in("fr", "inventory-sorter-default-base"), "Sorted");
    let pre_trash_fr = inventory_name(aurora_i18n::text_in("fr", "inventory-sorter-pre-trash"), "Pre-Trash");
    add_folder(&mut inv, 50, 1, &sorted_fr);
    add_folder(&mut inv, 51, 50, "Furniture");
    add_item(&mut inv, 52, 51, "Old chair", OLD);
    add_item(&mut inv, 53, 50, "Old lamp", OLD);
    add_folder(&mut inv, 60, 1, &pre_trash_fr);
    add_item(&mut inv, 61, 60, "Old freebie", OLD);
    let prefs = Preferences {
        cleanup: Cleanup {
            include_sorted: true,
            ..before_2020()
        },
        ..Default::default()
    };
    let plan = plan_cleanup(&inv, &prefs, &prefs.cleanup, id(1), today(), &HashSet::new(), &HashSet::new());
    assert!(plan.rows.iter().all(|r| r.id != id(50) && r.id != id(60)), "never whole");
    assert!(plan.rows.iter().all(|r| r.id != id(61)), "Pre-Trash is never read");
    let rule = Preferences {
        rules: vec![Rule {
            words: sorted_fr.to_lowercase(),
            mode: Match::Any,
            pre_trash: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let scan = scan_rules(&inv, &rule, id(1), &HashSet::new(), &HashSet::new());
    assert!(!scan.found.contains_key(&id(50)));
}

/// A resumed run or a layout restore leaves what was excluded since.
#[test]
fn a_resumed_run_honours_folders_excluded_since() {
    let mut inv = fixture(&["Velvet Sofa"]);
    add_folder(&mut inv, 70, OBJECTS, "Keep this - FATPACK");
    let moves = vec![
        Move {
            id: id(100),
            folder: false,
            name: "Velvet Sofa".into(),
            from: id(OBJECTS),
            to: Dest::Path(vec!["Furniture".into()]),
            label: String::new(),
            unpacker: false,
            restore: false,
            from_trash: false,
            snapshot: false,
            only_if_empty: false,
        },
        Move {
            id: id(70),
            folder: true,
            name: "Keep this - FATPACK".into(),
            from: id(OBJECTS),
            to: Dest::Path(vec!["Outfits".into()]),
            label: String::new(),
            unpacker: false,
            restore: false,
            from_trash: false,
            snapshot: false,
            only_if_empty: false,
        },
    ];
    let prefs = Preferences {
        excluded: vec![id(70)],
        ..Default::default()
    };
    let plan = plan_saved(&inv, &moves, &prefs, &HashSet::new(), &HashSet::new());
    assert_eq!(plan.moves.iter().map(|m| m.id).collect::<Vec<_>>(), vec![id(100)]);
    assert!(plan.kept_list.iter().any(|(_, why)| *why == Kept::Excluded));
}

/// Names as creators really write them, that the vocabulary used to file
/// wrongly: status tags are no store, an apostrophe joins its word, a home
/// item gets no body, accented rule folders keep their letters, a store
/// tag with no letter the inventory keeps gets no bare Stores folder.
#[test]
fn status_tags_apostrophes_bodies_and_accents_are_read_right() {
    assert_eq!(store("(Mesh) Sweater"), None);
    assert_eq!(store("[DEMO] Velvet Gown"), None);
    // both ways: its parts (rules written before) and the joined word
    assert_eq!(words("Women's Boots"), vec!["women", "s", "womens", "boots"]);
    assert_eq!(sorted_as("Hourglass Vase", 6, &[]), cat(Category::Decor));
    assert_eq!(sorted_as("Bed with 50 poses", 6, &[]), cat(Category::Furniture));
    assert_eq!(split_path("Robes d'été/Café"), vec!["Robes d'ete".to_owned(), "Cafe".to_owned()]);
    // a letter the table lacks is left out, never a `?`; a part of such
    // letters only is dropped
    assert_eq!(
        split_path("Ładne rzeczy/桜屋/Shoes"),
        vec!["adne rzeczy".to_owned(), "Shoes".to_owned()]
    );
    // a rule's words are read joined: `women's` never meets every `'s`
    let rule = Rule {
        words: "women's".into(),
        mode: Match::Any,
        enabled: true,
        folder: "W".into(),
        ..Default::default()
    };
    assert!(rule.named(&words("Women's Boots")));
    assert!(!rule.named(&words("Kat's Dress")));
    assert!(!rule.named(&words("Women Boots")));
    // a phrase with an apostrophe word after another still meets it
    let phrase = Rule {
        words: "red women's".into(),
        mode: Match::Phrase,
        enabled: true,
        folder: "W".into(),
        ..Default::default()
    };
    assert!(phrase.named(&words("Red Women's Dress")));
    assert!(!phrase.named(&words("Red Dress for Women's")));
    assert!(!sorted_as("[桜屋] something", 6, &[]).contains(&stores_folder()));
    // a folder of the user's named with a dash is not a product by its store
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 80, 1, "Clothes - Winter");
    add_folder(&mut inv, 81, 80, "Coats");
    assert!(!product_folder_in(&inv, id(80), &HashSet::new()));
}

/// Undo and a layout restore are never stopped by an exclusion added since
/// (excluding Sorted, then Undo, must still put things back); a resumed
/// run is.
#[test]
fn exclusions_stop_a_resume_but_never_an_undo() {
    let mut inv = fixture(&["Velvet Sofa"]);
    add_folder(&mut inv, 50, 1, "Sorted");
    if let Some(it) = inv.items.get_mut(&id(100)) {
        it.parent = id(50);
    }
    let back = Move {
        id: id(100),
        folder: false,
        name: "Velvet Sofa".into(),
        from: id(50),
        to: Dest::Folder(id(OBJECTS)),
        label: String::new(),
        unpacker: false,
        restore: true,
        from_trash: false,
        snapshot: false,
        only_if_empty: false,
    };
    let prefs = Preferences {
        excluded: vec![id(50)],
        ..Default::default()
    };
    let plan = plan_saved(&inv, std::slice::from_ref(&back), &prefs, &HashSet::new(), &HashSet::new());
    assert_eq!(plan.moves.len(), 1, "Undo still puts it back");
    let resume = Move { restore: false, ..back };
    let plan = plan_saved(&inv, &[resume], &prefs, &HashSet::new(), &HashSet::new());
    assert!(plan.moves.is_empty(), "a resumed run leaves it");
}

/// A file's "except Sorted" Pre-Trash rule is not the user's plain one.
#[test]
fn pre_trash_rules_compare_by_words_and_where_they_look() {
    use super::share::import;
    let found = import(&format!("{}\nrule: any | old | pre-trash | except-sorted\n", super::share::HEADER));
    let mine = Preferences {
        rules: vec![Rule {
            words: "old".into(),
            mode: Match::Any,
            pre_trash: true,
            folder: "left over".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert_eq!(found.duplicates(&mine).0, vec![false]);
    let same = Preferences {
        rules: vec![Rule {
            skip_sorted: true,
            ..mine.rules[0].clone()
        }],
        ..Default::default()
    };
    assert_eq!(
        found.duplicates(&same).0,
        vec![true],
        "a stale folder on a Pre-Trash rule is ignored"
    );
}

fn move_to(inv: &Inventory, at: u128, to: Dest) -> Move {
    Move {
        id: id(at),
        folder: inv.folders.contains_key(&id(at)),
        name: String::new(),
        from: actions::parent(inv, id(at)).expect("a parent"),
        to,
        label: String::new(),
        unpacker: false,
        restore: false,
        from_trash: false,
        snapshot: false,
        only_if_empty: false,
    }
}

/// One batch never carries an entry into Pre-Trash's tree by the back door:
/// an item sent into a folder that the same batch sends to Pre-Trash waits
/// for the next batch, where its destination is checked as it then is.
#[test]
fn one_batch_never_slips_an_entry_into_pre_trash() {
    let mut inv = fixture(&["Velvet chair"]);
    add_folder(&mut inv, 70, OBJECTS, "demo freebies");
    add_folder(&mut inv, 71, 70, "sub");
    let mut queue: VecDeque<Move> = [move_to(&inv, 100, Dest::Folder(id(71))), move_to(&inv, 70, Dest::PreTrash)].into();
    let none = HashSet::new();
    let mut skipped = 0;
    let Step::Change(_, sent) = step(&mut inv, "Sorted", &mut queue, &none, &none, &mut skipped) else {
        panic!("a batch")
    };
    assert_eq!(sent.len(), 1, "the folder waits for the next batch");
    assert_eq!(queue.len(), 1);
}

/// A folder sent to My Inventory's root never takes a name the viewer or
/// Auto-sort reads there.
#[test]
fn a_folder_never_lands_at_the_root_with_a_viewer_name() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 70, OBJECTS, "Archive");
    add_folder(&mut inv, 71, 70, "#RLV");
    let mut queue: VecDeque<Move> = [move_to(&inv, 71, Dest::Folder(id(1)))].into();
    let none = HashSet::new();
    let mut skipped = 0;
    assert!(matches!(
        step(&mut inv, "Sorted", &mut queue, &none, &none, &mut skipped),
        Step::Done
    ));
    assert_eq!(skipped, 1);
}

/// A folder whose contents are not loaded never moves or goes to the Trash
/// whole (a worn item could be inside, unseen): it is asked for and waits.
#[test]
fn a_folder_not_loaded_waits_before_it_moves_whole() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 70, OBJECTS, "Ryu Jeans - FATPACK");
    add_folder(&mut inv, 71, 70, "Colors");
    if let Some(f) = inv.folders.get_mut(&id(71)) {
        f.state = FetchState::Unknown;
    }
    let prefs = Preferences {
        folders: true,
        ..Default::default()
    };
    let plan = plan(&inv, id(OBJECTS), &prefs, &HashSet::new(), &HashSet::new());
    assert!(plan.moves.iter().all(|m| m.id != id(70)));
    assert!(plan.unfetched.contains(&id(71)), "asked for");
    let holds = actions::worn_ancestors(&inv, &HashSet::new());
    assert!(!trash_ready(&inv, id(70), id(OBJECTS), &prefs, &HashSet::new(), &holds));
}

/// A restore move only goes back to a folder: one read from a run file
/// edited by hand cannot send something along a path or to Pre-Trash.
#[test]
fn a_restore_never_goes_along_a_path_or_to_pre_trash() {
    let mut inv = fixture(&["Velvet Sofa"]);
    let mut odd = move_to(&inv, 100, Dest::PreTrash);
    odd.restore = true;
    let mut queue: VecDeque<Move> = [odd].into();
    let none = HashSet::new();
    let mut skipped = 0;
    assert!(matches!(
        step(&mut inv, "Sorted", &mut queue, &none, &none, &mut skipped),
        Step::Done
    ));
    assert_eq!(skipped, 1);
}

/// Two store names with no Latin letter are compared as written, never as
/// two empty names.
#[test]
fn stores_with_no_latin_letter_compare_as_written() {
    assert!(!same_store("Котик", "Мишка"));
    assert!(same_store("Котик", "котик"));
    assert!(same_store("Café", "Cafe"));
}

/// The folders the user lists as their own are opened and sorted whatever
/// their name, at any depth, and never move whole; a default folder or My
/// Inventory listed by hand counts for nothing.
#[test]
fn listed_own_folders_are_opened_never_moved_whole() {
    let none = HashSet::new();
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 700, 1, "Hair Collection");
    add_folder(&mut inv, 701, 700, "Long Waves");
    add_item(&mut inv, 801, 700, "Bob Cut Hair", 0);
    add_folder(&mut inv, 702, 1, "Hair Fatpacks");
    add_folder(&mut inv, 703, 702, "Colors");
    add_item(&mut inv, 802, 702, "Pixie Hair", 0);
    add_folder(&mut inv, 704, 1, "Lingerie Drawer");
    add_item(&mut inv, 803, 704, "Lace Bra", 0);
    // deeper than a whole sort opens on its own
    add_folder(&mut inv, 710, 1, "Home");
    add_folder(&mut inv, 711, 710, "Rooms");
    add_folder(&mut inv, 712, 711, "Upstairs");
    add_folder(&mut inv, 713, 712, "Bedroom");
    add_folder(&mut inv, 714, 713, "Corner");
    add_folder(&mut inv, 715, 714, "Decor Ideas");
    add_folder(&mut inv, 716, 715, "Cozy Lamp");
    add_item(&mut inv, 804, 715, "Glass Vase", 0);
    let listed = [700, 702, 704, 715];
    let prefs = Preferences {
        own: listed.iter().map(|n| id(*n)).collect(),
        ..Preferences::default()
    };
    let sources = whole_sources(&inv, &prefs);
    for n in listed {
        assert!(sources.contains(&id(n)), "listed folder {n} is opened");
    }
    let plan = plan_whole(&inv, &prefs, &none, &none);
    let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
    for n in [700, 702, 704, 710, 711, 712, 713, 714, 715] {
        assert!(!moved.contains(&id(n)), "folder {n} never moves whole");
    }
    for n in [801, 802, 803, 804] {
        assert!(moved.contains(&id(n)), "item {n} in a listed folder is sorted");
    }
    // My Inventory, a default folder, the Trash: never one of the user's own
    for n in [1, OBJECTS, 11] {
        assert!(!own_allowed(&inv, &prefs, id(n)), "folder {n} cannot be listed");
    }
    let refused = Preferences {
        own: vec![id(1), id(OBJECTS), id(11)],
        ..Preferences::default()
    };
    assert_eq!(
        whole_sources(&inv, &refused),
        whole_sources(&inv, &Preferences::default()),
        "a refused folder changes nothing"
    );
    // a folder in the sort folder cannot be listed either
    let base = Preferences::default().base_name();
    add_folder(&mut inv, 720, 1, &base);
    add_folder(&mut inv, 721, 720, "Hair Collection");
    assert!(!own_allowed(&inv, &prefs, id(721)), "inside the sort folder");
}

/// Deliveries audit 32 found opened: a pose pack with its props, hair with
/// its bang versions, a head or a body with its HUD and plural extras, a
/// vehicle with its rez box and riders' poses. Each stays whole.
#[test]
fn deliveries_with_props_bangs_huds_and_rez_boxes_stay_whole() {
    let none = HashSet::new();
    let layouts: [(&str, &[&str]); 9] = [
        ("Aster Photo Poses", &["Singles", "Couples", "Props"]),
        ("Kaida Hair", &["Bangs", "No Bangs", "HUD"]),
        ("Coral Hair", &["With Bangs", "No Bangs"]),
        ("Juno Hair", &["Bangs", "Without Bangs"]),
        ("Orion Head", &["HUDs", "Shapes", "Skins"]),
        ("Marlowe Dress Pack", &["Dresses", "HUD"]),
        ("Elara Body", &["HUDs", "Shapes", "Skins"]),
        ("Thunderbolt Motorcycle", &["Rez Box", "Riders Poses", "HUD"]),
        ("Sapphire Yacht", &["Rez Box", "Passenger Poses", "Textures"]),
    ];
    for (name, subs) in layouts {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        let mut inside = Vec::new();
        for (n, sub) in subs.iter().enumerate() {
            let folder = 901 + n as u128;
            add_folder(&mut inv, folder, 900, sub);
            add_item(&mut inv, 950 + n as u128, folder, &format!("{name} {sub}"), 0);
            inside.extend([folder, 950 + n as u128]);
        }
        let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
        let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
        assert!(!inside.iter().any(|n| moved.contains(&id(*n))), "{name} is never opened");
    }
    // a shelf of kinds stays a shelf
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 900, 1, "Fantasy");
    add_folder(&mut inv, 901, 900, "Heads");
    add_folder(&mut inv, 902, 900, "Skins");
    add_folder(&mut inv, 903, 900, "HUD");
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    assert!(!plan.moves.iter().any(|m| m.id == id(900)), "Fantasy is a shelf");
}

/// A folder listed in My folders since a run was planned stays on Resume; a
/// Pre-Trash word rule and Clean up never take one whole.
#[test]
fn listed_own_folders_stay_on_resume_pre_trash_and_clean_up() {
    let none = HashSet::new();
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 700, 1, "Old Hair Collection");
    add_item(&mut inv, 801, 700, "Old Bob Hair", date_start("2015-01").expect("date"));
    let prefs = Preferences {
        own: vec![id(700)],
        rules: vec![Rule {
            words: "old".into(),
            mode: Match::Any,
            pre_trash: true,
            ..Default::default()
        }],
        ..Preferences::default()
    };
    let resume = Move {
        id: id(700),
        folder: true,
        name: "Old Hair Collection".into(),
        from: id(1),
        to: Dest::Path(vec!["Hair".into()]),
        label: String::new(),
        unpacker: false,
        restore: false,
        from_trash: false,
        snapshot: false,
        only_if_empty: false,
    };
    let plan = plan_saved(&inv, std::slice::from_ref(&resume), &prefs, &none, &none);
    assert!(plan.moves.is_empty(), "a resumed run leaves a listed folder");
    assert!(plan.kept_list.iter().any(|(_, k)| *k == Kept::Own));
    let undo = Move { restore: true, ..resume };
    assert_eq!(plan_saved(&inv, &[undo], &prefs, &none, &none).moves.len(), 1, "Undo puts it back");
    let plan = plan_whole(&inv, &prefs, &none, &none);
    let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
    assert!(!moved.contains(&id(700)), "a Pre-Trash word rule never takes it whole");
    assert!(moved.contains(&id(801)), "its old item is taken alone");
    let cleanup = plan_cleanup(&inv, &prefs, &before_2020(), inv.root, today(), &none, &none);
    assert!(!cleanup.rows.iter().any(|r| r.id == id(700)), "Clean up takes its items one by one");
    assert!(cleanup.rows.iter().any(|r| r.id == id(801)));
}

/// A folder listed inside a delivery opens only itself: the delivery keeps
/// its own contents and stays in place. An exclusion wins over the list, and
/// a listed folder chosen by hand stays too.
#[test]
fn a_listed_folder_inside_a_delivery_opens_only_itself() {
    let none = HashSet::new();
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 700, 1, "Velvet Gown");
    add_folder(&mut inv, 701, 700, "HUD");
    add_item(&mut inv, 801, 701, "Velvet Gown HUD", 0);
    add_item(&mut inv, 802, 700, "Velvet Gown Maitreya", 0);
    add_folder(&mut inv, 702, 700, "My Edits");
    add_item(&mut inv, 803, 702, "Lace Bra", 0);
    let prefs = Preferences {
        own: vec![id(702)],
        ..Preferences::default()
    };
    let plan = plan_whole(&inv, &prefs, &none, &none);
    let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
    assert!(moved.contains(&id(803)), "the listed folder is sorted");
    for n in [700, 701, 702, 801, 802] {
        assert!(!moved.contains(&id(n)), "the delivery keeps {n}");
    }
    // excluded and listed: excluded wins, the delivery stays in place,
    // unopened
    let excluded = Preferences {
        excluded: vec![id(702)],
        ..prefs.clone()
    };
    assert!(!whole_sources(&inv, &excluded).contains(&id(700)));
    assert!(!whole_sources(&inv, &excluded).contains(&id(702)));
    // chosen by hand: kept, said so
    let plan = plan_entries(&inv, &[id(700), id(702)], &prefs, &none, &none);
    assert!(plan.moves.is_empty(), "never moved whole");
    assert_eq!(plan.kept_list.iter().filter(|(_, k)| *k == Kept::Own).count(), 2);
    // a Pre-Trash word rule and Clean up read only the listed folder inside
    // the delivery
    let old = date_start("2015-01").expect("date");
    add_item(&mut inv, 804, 700, "Old Velvet Gown Belt", old);
    add_item(&mut inv, 805, 702, "Old Lace Slip", old);
    let rule = Preferences {
        rules: vec![Rule {
            words: "old".into(),
            mode: Match::Any,
            pre_trash: true,
            ..Default::default()
        }],
        ..prefs.clone()
    };
    let moved: HashSet<Uuid> = plan_whole(&inv, &rule, &none, &none).moves.iter().map(|m| m.id).collect();
    assert!(
        moved.contains(&id(805)) && !moved.contains(&id(804)),
        "only the listed folder is read"
    );
    let cleanup = plan_cleanup(&inv, &prefs, &before_2020(), inv.root, today(), &none, &none);
    assert!(cleanup.rows.iter().any(|r| r.id == id(805)), "Clean up reads the listed folder");
    assert!(
        !cleanup.rows.iter().any(|r| [700, 701, 804].iter().any(|n| r.id == id(*n))),
        "the delivery stays whole"
    );
}

/// Shelves audit 33 found moved whole (a plural name beside a HUD or a body
/// folder), shelves of other stores' deliveries beside a `Rez Box`, and a
/// kitchen with its `Decor Only` version kept whole.
#[test]
fn plural_shelves_open_and_a_kitchen_with_its_decor_only_stays_whole() {
    let none = HashSet::new();
    let shelves: [(&str, &[&str]); 14] = [
        ("Work Outfits", &["HUD", "Dresses", "Tops"]),
        ("Work Outfits", &["Maitreya", "Reborn", "Tops"]),
        ("Gym Outfits", &["HUDs", "Tops"]),
        ("Fantasy Heads", &["HUDs", "Skins", "Shapes"]),
        ("Anime Heads", &["Catwa", "Shapes"]),
        ("Mens Shirts", &["Gianni", "Jake", "Jackets"]),
        ("Couple Poses", &["HUD", "Props", "Poses"]),
        ("Elf Skins", &["Lelutka", "Skins"]),
        ("Jewelry Sets", &["Necklaces", "Earrings", "HUD"]),
        ("Lingerie Sets", &["Maitreya", "Bras"]),
        ("Lashes", &["Lelutka", "Catwa"]),
        ("Eyebrows", &["Lelutka", "Catwa"]),
        ("Cars", &["Rez Box", "[Zoom] Bike", "Tempest Sport Car"]),
        ("AOs", &["HUD", "Velvet Dreams AO", "[Mila] AO"]),
    ];
    for (name, subs) in shelves {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        for (n, sub) in subs.iter().enumerate() {
            add_folder(&mut inv, 901 + n as u128, 900, sub);
        }
        let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
        assert!(!plan.moves.iter().any(|m| m.id == id(900)), "{name} never moves whole");
        assert!(whole_sources(&inv, &Preferences::default()).contains(&id(900)), "{name} is opened");
    }
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 900, 1, "Hollis Kitchen");
    for (n, sub) in ["Cabinets", "Appliances", "Decor Only"].iter().enumerate() {
        add_folder(&mut inv, 901 + n as u128, 900, sub);
        add_item(&mut inv, 950 + n as u128, 901 + n as u128, &format!("Hollis {sub}"), 0);
    }
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
    assert!(
        ![901, 902, 903, 950, 951, 952].iter().any(|n| moved.contains(&id(*n))),
        "the kitchen is never opened"
    );
}

/// Audit 34: a set named for one thing, a delivery holding its own store's
/// tagged folder stay whole; shelves of sets or packs open; ring and shape
/// names keep their category beside the new makeup and ear words.
#[test]
fn sets_and_own_store_tags_stay_whole_and_shelves_of_packs_open() {
    let none = HashSet::new();
    let whole: [(&str, &[&str]); 3] = [
        ("Kitsune Ears Set", &["HUD", "Ears"]),
        ("Kitsune Ears Set", &["HUD", "Ears", "Tails"]),
        ("Mila - Photo Studio", &["Props", "[Mila] Poses"]),
    ];
    for (name, subs) in whole {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        let mut inside = Vec::new();
        for (n, sub) in subs.iter().enumerate() {
            add_folder(&mut inv, 901 + n as u128, 900, sub);
            add_item(&mut inv, 950 + n as u128, 901 + n as u128, &format!("Kitsune {sub}"), 0);
            inside.extend([901 + n as u128, 950 + n as u128]);
        }
        let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
        let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
        assert!(!inside.iter().any(|n| moved.contains(&id(*n))), "{name} is never opened");
    }
    let shelves: [(&str, &[&str]); 3] = [
        ("Makeup Sets", &["Lelutka", "Catwa"]),
        ("Dress Packs", &["Maitreya", "HUD"]),
        ("Head Packs", &["HUDs", "Skins", "Shapes"]),
    ];
    for (name, subs) in shelves {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        for (n, sub) in subs.iter().enumerate() {
            add_folder(&mut inv, 901 + n as u128, 900, sub);
        }
        let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
        assert!(!plan.moves.iter().any(|m| m.id == id(900)), "{name} never moves whole");
    }
    assert_eq!(path("Brow Ring"), path("Silver Ring"));
    assert_eq!(path("Lash Ring"), path("Silver Ring"));
    assert_eq!(path("Eyebrow Piercing"), path("Brow Piercing"));
    assert_eq!(path("Elf Ear Shape"), path("Elf Shape"));
}

/// Audit 35: sets and collections of one thing stay whole, and a store's
/// own tag written another way is still its own.
#[test]
fn sets_of_one_thing_and_own_store_spellings_stay_whole() {
    let none = HashSet::new();
    let whole: [(&str, &[&str]); 7] = [
        ("Kyoto Kimono Sets", &["Kimono", "Obi", "HUD"]),
        ("Vesper Ring Sets", &["Gold", "Silver"]),
        ("Vesper Earring Sets", &["Gold", "Silver", "HUD"]),
        ("Opal Jewelry Collections", &["Necklaces", "HUD"]),
        ("Mila Rose - Photo Studio", &["Props", "[MilaRose] Poses"]),
        ("Mila-Rose - Photo Studio", &["Props", "[Mila Rose] Poses"]),
        ("Bloom&Co - Garden Gazebo", &["Props", "[Bloom & Co] Plants"]),
    ];
    for (name, subs) in whole {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        let mut inside = Vec::new();
        for (n, sub) in subs.iter().enumerate() {
            add_folder(&mut inv, 901 + n as u128, 900, sub);
            add_item(&mut inv, 950 + n as u128, 901 + n as u128, &format!("Item {sub}"), 0);
            inside.extend([901 + n as u128, 950 + n as u128]);
        }
        let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
        let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
        assert!(!inside.iter().any(|n| moved.contains(&id(*n))), "{name} is never opened");
    }
    assert!(same_store("Mila Rose", "Mila_Rose") && same_store("MILA.", "mila"));
    assert!(!same_store("Mila", "Milan"));
}

/// Audit 36: a shelf of sets holding products named their own way opens;
/// `A&B` and `AB` are two stores.
#[test]
fn shelves_of_sets_open_and_an_ampersand_names_another_store() {
    let none = HashSet::new();
    let shelves: [(&str, &[&str]); 5] = [
        ("Best Ring Sets", &["Vesper", "Opal"]),
        ("Female Pose Packs", &["Sunny", "Breeze", "Lazy Day"]),
        ("Long Hair Packs", &["Ava", "Bree"]),
        ("Office Outfit Bundles", &["Monday", "Tuesday"]),
        ("Bridal Jewelry Collections", &["Vesper", "Opal"]),
    ];
    for (name, subs) in shelves {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        for (n, sub) in subs.iter().enumerate() {
            add_folder(&mut inv, 901 + n as u128, 900, sub);
        }
        let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
        assert!(!plan.moves.iter().any(|m| m.id == id(900)), "{name} never moves whole");
    }
    assert!(!same_store("A&B", "AB"));
    assert!(same_store("Bloom&Co", "Bloom & Co"));
}

/// Audit 37: a set's versions by colour, gem or season keep it whole; one
/// colour folder beside products named their own way is a shelf's.
#[test]
fn sets_by_colour_or_season_stay_whole_and_a_lone_colour_is_a_shelf() {
    let none = HashSet::new();
    let whole: [(&str, &[&str]); 5] = [
        ("Vesper Ring Sets", &["Diamond", "Emerald", "Ruby"]),
        ("Kyoto Kimono Sets", &["Red", "Blue", "Floral"]),
        ("Nimbus Hair Packs", &["Blondes", "Reds", "Naturals"]),
        ("Opal Jewelry Collections", &["Fall 2024", "Winter 2024"]),
        ("Wren Pose Packs", &["Vol 1", "Vol 2"]),
    ];
    for (name, subs) in whole {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        let mut inside = Vec::new();
        for (n, sub) in subs.iter().enumerate() {
            add_folder(&mut inv, 901 + n as u128, 900, sub);
            add_item(&mut inv, 950 + n as u128, 901 + n as u128, &format!("{name} {sub}"), 0);
            inside.extend([901 + n as u128, 950 + n as u128]);
        }
        let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
        let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
        assert!(!inside.iter().any(|n| moved.contains(&id(*n))), "{name} is never opened");
    }
    let shelves: [(&str, &[&str]); 3] = [
        ("Best Ring Sets", &["Gold", "Vesper", "Opal"]),
        ("Fave Hair Packs", &["Black", "Ava", "Bree"]),
        ("Fave Jewelry Collections", &["Silver", "Vesper"]),
    ];
    for (name, subs) in shelves {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        for (n, sub) in subs.iter().enumerate() {
            add_folder(&mut inv, 901 + n as u128, 900, sub);
        }
        let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
        assert!(!plan.moves.iter().any(|m| m.id == id(900)), "{name} never moves whole");
    }
    assert!(same_store("Bloom & Co", "Bloom Co.") && !same_store("A&B", "AB"));
}

/// A move refused since the plan (a folder excluded during the run) is
/// skipped before anything below it is loaded.
#[test]
fn a_refused_folder_move_is_skipped_before_its_tree_loads() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 700, 1, "Huge Folder");
    add_folder(&mut inv, 701, 700, "Never Read");
    inv.folders.get_mut(&id(701)).expect("folder").state = FetchState::Unknown;
    let mut queue: VecDeque<Move> = VecDeque::from([Move {
        id: id(700),
        folder: true,
        name: "Huge Folder".into(),
        from: id(1),
        to: Dest::Path(vec!["Stuff".into()]),
        label: String::new(),
        unpacker: false,
        restore: false,
        from_trash: false,
        snapshot: false,
        only_if_empty: false,
    }]);
    let none = HashSet::new();
    let mut skipped = 0;
    let step = step_with(
        &mut inv,
        SortBase { id: None, name: "Sorted" },
        &mut queue,
        &none,
        &none,
        &mut skipped,
        &|_, m| m.id == id(700),
    );
    assert!(!matches!(step, Step::Loading), "never loaded only to be skipped");
    assert_eq!((skipped, queue.len()), (1, 0));
    assert_eq!(inv.folders[&id(701)].state, FetchState::Unknown, "nothing asked for");
}

/// Audit 38: a set with one colour of its own or a `Demo` folder beside its
/// finishes stays whole; a kind with a year beside products named their own
/// way is still a shelf's.
#[test]
fn sets_with_one_odd_folder_stay_whole_and_a_dated_kind_is_a_shelf() {
    let none = HashSet::new();
    let whole: [(&str, &[&str]); 5] = [
        ("Halcyon Dress Packs", &["Black", "White", "Burgundy"]),
        ("Rivet Boot Packs", &["Black", "Brown", "Plum"]),
        ("Vesper Ring Sets", &["Gold", "Silver", "Demo"]),
        ("Kyoto Kimono Sets", &["Sakura", "Hanami", "HUD 2"]),
        ("Speedster Car Packs", &["Roadster", "Coupe", "Scripts 2"]),
    ];
    for (name, subs) in whole {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        let mut inside = Vec::new();
        for (n, sub) in subs.iter().enumerate() {
            add_folder(&mut inv, 901 + n as u128, 900, sub);
            add_item(&mut inv, 950 + n as u128, 901 + n as u128, &format!("{name} {sub}"), 0);
            inside.extend([901 + n as u128, 950 + n as u128]);
        }
        let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
        let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
        assert!(!inside.iter().any(|n| moved.contains(&id(*n))), "{name} is never opened");
    }
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 900, 1, "Best Ring Sets");
    for (n, sub) in ["Ring 2", "Vesper", "Opal"].iter().enumerate() {
        add_folder(&mut inv, 901 + n as u128, 900, sub);
    }
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    assert!(!plan.moves.iter().any(|m| m.id == id(900)), "a shelf never moves whole");
}

/// Refused moves are skipped a few batches' worth a frame, never a whole
/// 400,000-move queue at once.
#[test]
fn refused_moves_are_skipped_a_few_batches_a_frame() {
    let mut inv = fixture(&["Lace Bra"]);
    let one = Move {
        id: id(100),
        folder: false,
        name: "Lace Bra".into(),
        from: id(OBJECTS),
        to: Dest::Path(vec!["Lingerie".into()]),
        label: String::new(),
        unpacker: false,
        restore: false,
        from_trash: false,
        snapshot: false,
        only_if_empty: false,
    };
    let mut queue: VecDeque<Move> = std::iter::repeat_n(one, 4 * BATCH + 5).collect();
    let none = HashSet::new();
    let mut skipped = 0;
    let step = step_with(
        &mut inv,
        SortBase { id: None, name: "Sorted" },
        &mut queue,
        &none,
        &none,
        &mut skipped,
        &|_, _| true,
    );
    assert!(matches!(step, Step::Wait));
    assert_eq!((skipped, queue.len()), (4 * BATCH, 5));
}

/// Moves no longer where the run left them (an Undo of entries all moved by
/// hand since) are skipped a few batches' worth a frame too.
#[test]
fn stale_moves_are_skipped_a_few_batches_a_frame() {
    let mut inv = fixture(&["Lace Bra"]);
    let gone = Move {
        id: id(100),
        folder: false,
        name: "Lace Bra".into(),
        // (not where it is now)
        from: id(1),
        to: Dest::Path(vec!["Lingerie".into()]),
        label: String::new(),
        unpacker: false,
        restore: false,
        from_trash: false,
        snapshot: false,
        only_if_empty: false,
    };
    let mut queue: VecDeque<Move> = std::iter::repeat_n(gone, 4 * BATCH + 5).collect();
    let none = HashSet::new();
    let mut skipped = 0;
    let step = step_with(
        &mut inv,
        SortBase { id: None, name: "Sorted" },
        &mut queue,
        &none,
        &none,
        &mut skipped,
        &|_, _| false,
    );
    assert!(matches!(step, Step::Wait));
    assert_eq!((skipped, queue.len()), (4 * BATCH, 5));
}

/// Unpacked deliveries whose product no vocabulary names stay whole: an
/// unpacker's marks, a version tag, or their brand on every item. A shelf
/// of other creators' things is still opened.
#[test]
fn unpacked_deliveries_stay_whole_whatever_they_sell() {
    let none = HashSet::new();
    let layouts: [(&str, &[&str]); 9] = [
        ("Zephyr Gizmo (unpacked)", &["Zephyr Gizmo", "Zephyr Gizmo Guide"]),
        ("Quillon Thing [BOXED]", &["Quillon Thing", "Readme"]),
        ("Brindle Combat System v4", &["Brindle Help", "Brindle Sword", "Brindle Dodge"]),
        ("Tinker Style (1.0.4)", &["Setup", "Clothing.zip", "Skin.zip"]),
        (
            "NORRIS. Track Kit PACK",
            &["NORRIS. Track Top", "NORRIS. Track Bottom", "NORRIS FLAGSHIP"],
        ),
        (
            "Rascal Polo by Lumen",
            &["Polo XS by Lumen", "Polo M by Lumen", "Vintage Polo (wear me)"],
        ),
        ("Quiet Faders (Wear to Auto-Unpack)", &["Fader Left", "Fader Right"]),
        ("Brindle Combat System v.4", &["Help", "Sword"]),
        ("Kitty Makeup - Cateyes I - Pack 1", &["Kitty Cateyes Light", "Kitty Cateyes Dark"]),
    ];
    for (name, items) in layouts {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        for (n, item) in items.iter().enumerate() {
            add_item(&mut inv, 950 + n as u128, 900, item, 0);
        }
        let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
        let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
        assert!(
            !(950..950 + items.len() as u128).any(|n| moved.contains(&id(n))),
            "{name} is never opened"
        );
    }
    // a shelf of other creators' things, a dated folder, `Unpacked` alone
    let shelves: [(&str, &[&str]); 3] = [
        ("Club Night", &["Zephyr Dance", "Quillon Spin", "Brindle Sway"]),
        ("10.12.2016", &["Zephyr Gizmo", "Quillon Thing"]),
        ("Unpacked", &["Zephyr Gizmo", "Quillon Thing"]),
    ];
    for (name, items) in shelves {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        for (n, item) in items.iter().enumerate() {
            add_item(&mut inv, 950 + n as u128, 900, item, 0);
        }
        assert!(whole_sources(&inv, &Preferences::default()).contains(&id(900)), "{name} is opened");
    }
}

/// Audit 41: a shelf's name in decoration is no store tag, and a pair kind
/// inside its own product (`Kira Boots` holding `Boots`) is no shelf.
#[test]
fn decorated_shelves_open_and_a_product_keeps_its_own_pair() {
    let none = HashSet::new();
    for name in ["~ Fave Rings ~", "[My Rings]", "*Fave Poses*"] {
        assert!(store(name).is_none(), "{name} is no store");
    }
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 900, 1, "Kira Boots");
    add_folder(&mut inv, 901, 900, "Boots");
    add_folder(&mut inv, 902, 900, "HUD");
    add_item(&mut inv, 950, 901, "Kira Boots Legacy", 0);
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
    assert!(!moved.contains(&id(950)) && !moved.contains(&id(901)), "Kira Boots is never opened");
}

/// Audit 42: a body's shelf, `Unpacked Stuff`, `Unboxed Items`, `Swimwear
/// Men` and a bracketed date are shelves, never one delivery.
#[test]
fn body_shelves_unpacked_stuff_and_dates_are_shelves() {
    let shelves: [(&str, &[&str]); 6] = [
        (
            "Maitreya Clothes",
            &["Hex Dress Maitreya", "Kira Top Lara X Maitreya", "Jeans Maitreya"],
        ),
        ("Unpacked Stuff", &["Zephyr Gizmo", "Quillon Thing"]),
        ("Unboxed Items", &["Zephyr Gizmo", "Quillon Thing"]),
        ("Swimwear Men", &["Zephyr Trunks", "Quillon Shorts"]),
        ("Christmas (12.2023)", &["Zephyr Gizmo", "Quillon Thing"]),
        ("Hunt [10.12.2016]", &["Zephyr Gizmo", "Quillon Thing"]),
    ];
    for (name, items) in shelves {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        for (n, item) in items.iter().enumerate() {
            add_item(&mut inv, 950 + n as u128, 900, item, 0);
        }
        assert!(whole_sources(&inv, &Preferences::default()).contains(&id(900)), "{name} is opened");
    }
    assert!(version_tag("Kira HUD [6.0]") && version_tag("Kira (1.0.4)") && version_tag("Kira v.4"));
    assert!(!version_tag("Hunt (10.12.2016)") && !version_tag("Gifts [24.12]") && !version_tag("Fair (12.2023)"));
}

/// Audit 43: a bracket of everyday words is no unpacker's mark, and a mark
/// beside shelf words only is the user's pile.
#[test]
fn everyday_brackets_and_marked_piles_are_shelves() {
    for name in [
        "Gifts (Gift Box)",
        "My Stuff (unpacked)",
        "To Unpack",
        "Dresses [Unpacked]",
        "Boxed Gifts",
    ] {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        add_item(&mut inv, 950, 900, "Zephyr Gizmo", 0);
        add_item(&mut inv, 951, 900, "Quillon Thing", 0);
        assert!(whole_sources(&inv, &Preferences::default()).contains(&id(900)), "{name} is opened");
    }
    for name in [
        "Kira Dress (Wear)",
        "Kira HUD [BOXED]",
        "Slapper {Unpacker/backup}",
        "Kira (add me)",
    ] {
        assert_eq!(delivery_mark(name), Some(Mark::Strong), "{name}");
    }
}

/// Audit 43: a shelf named with a shelf word keeps loose items of many
/// creators sharing its qualifier (`Group Gifts` holding `Kira Hat (Group
/// Gift)`).
#[test]
fn a_shelf_word_shelf_shares_no_brand() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 900, 1, "Group Gifts");
    add_item(&mut inv, 950, 900, "Velmora Dress GROUP GIFT", 0);
    add_item(&mut inv, 951, 900, "Kira Hat (Group Gift)", 0);
    add_item(&mut inv, 952, 900, "Lumen Group Gift Boots", 0);
    assert!(whole_sources(&inv, &Preferences::default()).contains(&id(900)));
}

/// Real layouts: a brand-first folder with `My` in its product's name, a
/// box's folder holding just the product, and `<product> Unboxed` stay
/// whole; `My Stuff` and `Unboxed Items` stay piles.
#[test]
fn brand_first_box_folders_and_unboxed_products_stay_whole() {
    let none = HashSet::new();
    let layouts: [(&str, &[&str]); 3] = [
        (
            "Zorin. My Little Book",
            &["Zorin. My Little Book - Curvy", "Zorin. My Little Book", "Zorin Mainstore"],
        ),
        ("Quarry mesh BOX", &["Quarry mesh"]),
        (
            "Brindle Old Milk Cans Unboxed",
            &["Brindle Old Milk Can", "Brindle Old Milk Can - rusty"],
        ),
    ];
    for (name, items) in layouts {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        for (n, item) in items.iter().enumerate() {
            add_item(&mut inv, 950 + n as u128, 900, item, 0);
        }
        let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
        let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
        assert!(
            !(950..950 + items.len() as u128).any(|n| moved.contains(&id(n))),
            "{name} is never opened"
        );
    }
    for name in ["My Stuff", "Unboxed Items"] {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        add_item(&mut inv, 950, 900, "Zephyr Gizmo", 0);
        add_item(&mut inv, 951, 900, "Quillon Thing", 0);
        assert!(whole_sources(&inv, &Preferences::default()).contains(&id(900)), "{name} is opened");
    }
}

/// A folder the server failed to give is unknown, never empty: a folder
/// holding one never moves whole, by a plan or a run, and Clean up never
/// takes it as empty.
#[test]
fn a_folder_that_failed_to_load_is_unknown_never_empty() {
    let none = HashSet::new();
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 900, 1, "Zephyr Gizmo (unpacked)");
    add_folder(&mut inv, 901, 900, "failed");
    inv.folders.get_mut(&id(901)).expect("folder").state = FetchState::Failed;
    add_item(&mut inv, 950, 900, "Zephyr Gizmo", 0);
    // chosen by hand: kept, said so
    let plan = plan_entries(&inv, &[id(900)], &Preferences::default(), &none, &none);
    assert!(plan.moves.is_empty());
    assert_eq!(plan.kept_list.iter().filter(|(_, k)| *k == Kept::Unreadable).count(), 1);
    // a run's move of it is skipped, never sent
    let mut queue: VecDeque<Move> = VecDeque::from([Move {
        id: id(900),
        folder: true,
        name: "Zephyr Gizmo (unpacked)".into(),
        from: id(1),
        to: Dest::Path(vec!["Stuff".into()]),
        label: String::new(),
        unpacker: false,
        restore: false,
        from_trash: false,
        snapshot: false,
        only_if_empty: false,
    }]);
    let mut skipped = 0;
    let step = step_with(
        &mut inv,
        SortBase { id: None, name: "Sorted" },
        &mut queue,
        &none,
        &none,
        &mut skipped,
        &|_, _| false,
    );
    assert!(!matches!(step, Step::Change(..)) && skipped == 1);
    // Clean up's empty folders never take it
    let cleanup = Cleanup {
        empty_folders: true,
        ..before_2020()
    };
    let plan = plan_cleanup(&inv, &Preferences::default(), &cleanup, inv.root, today(), &none, &none);
    assert!(!plan.rows.iter().any(|r| r.id == id(901) || r.id == id(900)));
}

/// Audit 44: a shelf word before a dash is no store, holidays and genders
/// are no brand, a dated name is the user's, and a box's folder of one
/// product with one word of its own stays whole.
#[test]
fn shelf_dashes_holidays_genders_and_dates_are_shelves() {
    let none = HashSet::new();
    let shelves: [(&str, &[&str]); 6] = [
        ("Old - Dresses", &["Zephyr Gown", "Quillon Frock"]),
        ("Xmas 2023", &["Xmas Tree by Velmora", "Kira Xmas Sweater", "Lumen Xmas Lights"]),
        ("Xmas Decor", &["Zephyr Wreath", "Quillon Candle"]),
        ("Female", &["Velmora Top Female", "Kira Hair Female", "Lumen Boots (Female)"]),
        ("Swimwear Men", &["Zephyr Trunks", "Quillon Shorts"]),
        ("Zoe & Me 10.12.2016", &["Zoe's gift", "Zoe Birthday Card"]),
    ];
    for (name, items) in shelves {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        for (n, item) in items.iter().enumerate() {
            add_item(&mut inv, 950 + n as u128, 900, item, 0);
        }
        assert!(whole_sources(&inv, &Preferences::default()).contains(&id(900)), "{name} is opened");
    }
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 900, 1, "Harvester mesh BOX");
    add_item(&mut inv, 950, 900, "Harvester mesh table", 0);
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    assert!(!plan.moves.iter().any(|m| m.id == id(950)), "the box's folder stays whole");
}

/// Audit 45: a `v` version holding a year is no date.
#[test]
fn a_versioned_name_with_a_year_is_no_date() {
    let none = HashSet::new();
    for name in ["Kira Gizmo v1.0.2024", "Kira Gizmo (v1.0.2024)", "Kira Gizmo v2024.1"] {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        add_item(&mut inv, 950, 900, "Kira Gizmo", 0);
        add_item(&mut inv, 951, 900, "Kira Gizmo HUD", 0);
        let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
        let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
        assert!(!moved.contains(&id(950)) && !moved.contains(&id(951)), "{name} is never opened");
    }
}

/// The share's read of a large tree is done once nothing is left to come:
/// the Trash's tree (never asked for), a failed folder and one too large
/// for the server never hold it; a folder still on its way does.
#[test]
fn the_shared_read_ends_whatever_the_trash_and_failed_folders_hold() {
    let mut inv = fixture(&[]);
    for n in 0..2000u128 {
        add_folder(&mut inv, 10_000 + n, 1, &format!("Shelf {n}"));
    }
    // the Trash's folders are never read
    add_folder(&mut inv, 900, 11, "Deleted");
    inv.folders.get_mut(&id(900)).expect("folder").state = FetchState::Unknown;
    add_folder(&mut inv, 901, 1, "failed");
    inv.folders.get_mut(&id(901)).expect("folder").state = FetchState::Failed;
    add_folder(&mut inv, 902, 1, "Huge");
    inv.folders.get_mut(&id(902)).expect("folder").state = FetchState::Unknown;
    inv.too_big.insert(id(902));
    add_folder(&mut inv, 903, 901, "Cached child of failed read");
    add_folder(&mut inv, 904, 902, "Cached child of oversized folder");
    for n in [903, 904] {
        inv.folders.get_mut(&id(n)).expect("folder").state = FetchState::Unknown;
    }
    let progress = read_progress(&inv);
    assert_eq!(progress.pending, 0, "nothing left to come");
    assert_eq!(progress.failed, 2);
    assert!(progress.read >= 2000);
    // one still on its way holds it
    inv.folders.get_mut(&id(10_000)).expect("folder").state = FetchState::Fetching;
    assert_eq!(read_progress(&inv).pending, 1);
}

/// The share's read asks only the topmost unread folders (each read with
/// all it holds), never the Trash's nor a folder too large for the server.
#[test]
fn the_shared_read_asks_only_the_topmost_unread_folders() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 900, 1, "Shelf");
    add_folder(&mut inv, 901, 900, "Inner");
    add_folder(&mut inv, 902, 11, "Deleted");
    add_folder(&mut inv, 903, 1, "Huge");
    for n in [900, 901, 902, 903] {
        inv.folders.get_mut(&id(n)).expect("folder").state = FetchState::Unknown;
    }
    inv.too_big.insert(id(903));
    assert_eq!(unread_tops(&inv, 10), vec![id(900)]);
}

/// Audit 46: a gift or holiday shelf of other stores' deliveries named with
/// its word is opened.
#[test]
fn gift_and_holiday_shelves_of_deliveries_open() {
    let shelves: [(&str, &[&str]); 3] = [
        ("Group Gifts", &["Kira GROUP GIFT Boots", "Velmora Group Gift Dress"]),
        ("Christmas", &["Kira Christmas Dress", "Velmora Christmas Hat"]),
        ("Xmas 2024", &["Kira Xmas Dress", "Velmora Xmas Hat"]),
    ];
    for (name, subs) in shelves {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        for (n, sub) in subs.iter().enumerate() {
            add_folder(&mut inv, 901 + n as u128, 900, sub);
        }
        assert!(whole_sources(&inv, &Preferences::default()).contains(&id(900)), "{name} is opened");
    }
}

/// Audit 47: an occasion beside a kind names one product, its colours its
/// versions.
#[test]
fn an_occasion_product_keeps_its_colour_versions() {
    let none = HashSet::new();
    for (name, subs) in [
        ("Christmas Sweater", ["Christmas Sweater Black", "Christmas Sweater Red"]),
        ("Winter Boots", ["Winter Boots Black", "Winter Boots Brown"]),
    ] {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        for (n, sub) in subs.iter().enumerate() {
            add_folder(&mut inv, 901 + n as u128, 900, sub);
            add_item(&mut inv, 950 + n as u128, 901 + n as u128, sub, 0);
        }
        let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
        let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
        assert!(
            ![901, 902, 950, 951].iter().any(|n| moved.contains(&id(*n))),
            "{name} is never opened"
        );
    }
}

/// Audit 47: a product repeating its whole name, or its shelf word first,
/// in each part keeps them whole.
#[test]
fn products_repeating_their_name_or_shelf_word_stay_whole() {
    let none = HashSet::new();
    let layouts: [(&str, [&str; 2]); 5] = [
        ("Christmas Sweater", ["Christmas Sweater Reindeer Top", "Christmas Sweater HUD"]),
        (
            "Halloween Costume",
            ["Halloween Costume Witch Hat", "Halloween Costume Witch Dress"],
        ),
        ("Gift Starlight Outfit", ["Gift Starlight Top", "Gift Starlight Witch Hat"]),
        ("Easter Bunny Ears", ["Easter Bunny Ears HUD", "Easter Bunny Ears Rigged"]),
        ("Xmas Pajamas", ["Xmas Pajamas Maitreya", "Xmas Pajamas HUD"]),
    ];
    for (name, subs) in layouts {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        for (n, sub) in subs.iter().enumerate() {
            add_folder(&mut inv, 901 + n as u128, 900, sub);
            add_item(&mut inv, 950 + n as u128, 901 + n as u128, sub, 0);
        }
        let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
        let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
        assert!(
            ![901, 902, 950, 951].iter().any(|n| moved.contains(&id(*n))),
            "{name} is never opened"
        );
    }
}

/// Audit 48: a "<occasion> <kind>" shelf of other stores' tagged
/// deliveries is opened.
#[test]
fn an_occasion_kind_shelf_of_tagged_deliveries_opens() {
    for (name, subs) in [
        ("Christmas Sweater", ["[Kira] Christmas Sweater", "Velmora. Christmas Sweater"]),
        ("Halloween Ears", ["[Kira] Halloween Ears", "[Velmora] Halloween Ears"]),
    ] {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        for (n, sub) in subs.iter().enumerate() {
            add_folder(&mut inv, 901 + n as u128, 900, sub);
        }
        assert!(whole_sources(&inv, &Preferences::default()).contains(&id(900)), "{name} is opened");
    }
}

/// Audit 49: colour versions written before the whole name keep an
/// occasion product whole.
#[test]
fn colour_first_versions_keep_an_occasion_product_whole() {
    let none = HashSet::new();
    for (name, subs) in [
        ("Christmas Sweater", ["Red Christmas Sweater", "Green Christmas Sweater"]),
        ("Winter Boots", ["Black Winter Boots", "Brown Winter Boots"]),
    ] {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        for (n, sub) in subs.iter().enumerate() {
            add_folder(&mut inv, 901 + n as u128, 900, sub);
            add_item(&mut inv, 950 + n as u128, 901 + n as u128, sub, 0);
        }
        let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
        let moved: HashSet<Uuid> = plan.moves.iter().map(|m| m.id).collect();
        assert!(
            ![901, 902, 950, 951].iter().any(|n| moved.contains(&id(*n))),
            "{name} is never opened"
        );
    }
}

/// Audit 51: a colour or shelf-marker shelf of loose items, a pile of
/// demos and a part kind's gift shelf open; a worn thing's display piece
/// is decor or furniture.
#[test]
fn colour_shelves_demo_piles_and_display_pieces() {
    let none = HashSet::new();
    for (name, items) in [
        ("Black Dresses", ["Velmora Hex Dress Black", "Kira Gown Black"]),
        ("Shoes Backup", ["Velmora Pumps", "Kira Boots"]),
        ("Hair Demos", ["Calyx Bun Hair DEMO", "Wren Messy Bun DEMO"]),
    ] {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        for (n, item) in items.iter().enumerate() {
            add_item(&mut inv, 950 + n as u128, 900, item, 0);
        }
        let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
        assert!(!plan.moves.iter().any(|m| m.id == id(900)), "{name} never moves whole");
    }
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 900, 1, "Skin Gifts");
    add_folder(&mut inv, 901, 900, "Ilse Skin Gift");
    add_folder(&mut inv, 902, 900, "Kira Skin Gift");
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    assert!(!plan.moves.iter().any(|m| m.id == id(900)), "Skin Gifts never moves whole");
    assert_eq!(classify("Velvet Heels Decor", 6, 0).category, Some(Category::Decor));
    assert_eq!(classify("Shoe Rack", 6, 0).category, Some(Category::Furniture));
    assert_eq!(classify("Tattoo Chair", 6, 0).category, Some(Category::Furniture));
}

/// A real inventory of hunt prizes and gifts: a numbered hunt prize, a gift
/// from a store and a store's group gift are deliveries; shelves of gifts
/// and prizes stay shelves.
#[test]
fn hunt_prizes_and_gifts_from_a_store_stay_whole() {
    for name in [
        "THF14 025 of 110 Brindle Designs",
        "Gift from Zephyr Works",
        "Quillon March 2025 Group Gift",
        "Easter Basket (hunt item)",
    ] {
        assert_eq!(delivery_mark(name), Some(Mark::Strong), "{name}");
    }
    for name in ["Gifts", "Hunt Prizes", "Group Gifts 2024", "Hunt Gift", "Free Gift"] {
        assert_ne!(delivery_mark(name), Some(Mark::Strong), "{name}");
    }
}

/// Audit 52: a set whose folders are its parts moves whole; a shelf of
/// other stores' tagged pieces stays a shelf.
#[test]
fn a_set_of_untagged_parts_moves_whole() {
    let none = HashSet::new();
    for (name, parts) in [
        ("Boho Bedroom Set", ["Bed", "Night Table", "Floor Lamp"]),
        ("Vesper Bridal Set", ["Pearl Necklace", "Drop Earrings", "Crystal Tiara"]),
    ] {
        let mut inv = fixture(&[]);
        add_folder(&mut inv, 900, 1, name);
        for (n, part) in parts.iter().enumerate() {
            add_folder(&mut inv, 910 + n as u128, 900, part);
        }
        let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
        assert!(
            !plan.moves.iter().any(|m| (910..913).any(|n| m.id == id(n))),
            "{name} is never split"
        );
    }
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 900, 1, "Bedroom");
    add_folder(&mut inv, 910, 900, "[Vesper] Bed");
    add_folder(&mut inv, 911, 900, "[Kira] Lamp");
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    assert!(!plan.moves.iter().any(|m| m.id == id(900)), "a shelf never moves whole");
}

/// Real use: flexi is hair; makeup and a head's EvoX content are the
/// head's, never filed under a body named by one of its words.
#[test]
fn flexi_is_hair_and_makeup_is_never_a_bodys() {
    assert_eq!(classify("Velmora Flexi Bun", 6, 0).category, Some(Category::Hair));
    assert_eq!(classify("Velmora Flexi Skirt", 6, 0).category, Some(Category::Skirts));
    let shadow = classify("ZORIN, Isis Eyeshadow EvoX Bom", 6, 0);
    assert_eq!(shadow.category, Some(Category::Makeup));
    assert!(shadow.body.is_none(), "no body for makeup");
    let gloss = classify("Kira Lipgloss EvoX", 6, 0);
    assert_eq!(gloss.category, Some(Category::Makeup));
    assert!(gloss.body.is_none());
    let applier = classify("Kira Venus Freckles EvoX BOM", 6, 0);
    assert!(applier.body.is_none(), "an EvoX applier is the head's");
    // a body's clothes keep their body
    assert!(classify("Kira Dress Isis", 6, 0).body.is_some());
}

/// Real use: a permission tag is no body mod.
#[test]
fn a_permission_tag_is_no_body_mod() {
    assert_ne!(
        classify("Brindle - Autumn Dinner - Place Setting - White - NO MOD", 6, 0).category,
        Some(Category::BodyAddons)
    );
    assert_ne!(classify("Brindle Floor Lamp (Copy/Mod)", 6, 0).category, Some(Category::BodyAddons));
    assert_eq!(classify("Kira Belly Mod", 6, 0).category, Some(Category::BodyAddons));
}

/// Real use: a head's add-on is the head's; a decor set's glasses are decor.
#[test]
fn head_addons_and_decor_glasses() {
    let addon = classify("Kira Fangs Genus Addon", 6, 0);
    assert_eq!(addon.category, Some(Category::Heads));
    assert!(addon.body.is_none());
    assert_eq!(classify("Kira Belly Addon Reborn", 6, 0).category, Some(Category::BodyAddons));
    assert_eq!(
        classify("Brindle . Rosemary Kitchen Decor . Glasses 8", 6, 0).category,
        Some(Category::Decor)
    );
    assert_eq!(classify("Kira Round Glasses", 6, 0).category, Some(Category::Accessories));
}

/// Real use: platforms are shoes, a pose platform is furniture.
#[test]
fn platforms_are_shoes() {
    assert_eq!(classify("[Velmora] Daisie Platforms - noir", 6, 0).category, Some(Category::Shoes));
    assert_eq!(classify("Kira Platform Oxford Black Reborn", 6, 0).category, Some(Category::Shoes));
    assert_eq!(classify("Brindle Pose Platform", 6, 0).category, Some(Category::Furniture));
}

/// Real use: a store written in spaced letters is one store with its plain
/// spelling; its makeup reads as makeup.
#[test]
fn a_spaced_letter_store_is_one_store() {
    let spaced = store(". z o r i n a . // shoelace - Reborn (R)").expect("spaced tag");
    let plain = store("zorina - leg bandaids HUD").expect("plain tag");
    assert!(same_store(&spaced, &plain), "{spaced} / {plain}");
    assert_eq!(
        classify(". z o r i n a . // baby body blush - V4 (tint me!)", 6, 0).category,
        Some(Category::Makeup)
    );
}

/// A store hint for a spaced-letter store matches however it is typed.
#[test]
fn a_spaced_store_hint_matches_however_it_is_typed() {
    for typed in [". z o r i n a .", "z o r i n a", "zorina", "Zorina"] {
        let hints = [StoreHint {
            store: typed.into(),
            category: Category::Makeup,
            ..Default::default()
        }];
        let sorted = sorted_as(". z o r i n a . // shoelace - Reborn (R)", 6, &hints);
        assert!(sorted.contains(Category::Makeup.folder()), "{typed}: {sorted}");
    }
}

/// A copy for the duplicate finder: `asset` 0 is an asset id the server
/// did not give.
fn add_copy(inv: &mut Inventory, at: u128, parent: u128, name: &str, asset: u128, created: i64, copy: bool) {
    inv.add_items(vec![InvItem {
        id: id(at),
        parent: id(parent),
        name: name.into(),
        asset_type: 6,
        inv_type: 6,
        asset_id: if asset == 0 { Uuid::nil() } else { id(asset) },
        creator: id(5),
        owner: id(AGENT),
        owner_mask: if copy { 0x7fff_ffff } else { 0x7fff_ffff & !0x8000 },
        next_owner_mask: 0x0008_2000,
        created_at: created,
        ..Default::default()
    }]);
}

fn duplicates_of(inv: &Inventory, no_copy: bool) -> CleanupPlan {
    let cleanup = Cleanup {
        find: Find::Duplicates,
        no_copy_duplicates: no_copy,
        ..Default::default()
    };
    let none = HashSet::new();
    plan_duplicates(inv, &Preferences::default(), &cleanup, inv.root, &none, &none)
}

/// The duplicate finder lists every loose copy but one: the one already
/// filed, else the oldest; never an item whose asset id is unknown, a
/// no-copy item unless asked, nor two items that only share a name.
#[test]
fn the_duplicate_finder_keeps_one_copy_and_never_merges_unknown_items() {
    let mut inv = fixture(&[]);
    // three loose copies of one lamp (My Inventory and Objects): the oldest stays
    add_copy(&mut inv, 200, 1, "Brindle Lamp", 900, 300, true);
    add_copy(&mut inv, 201, OBJECTS, "Brindle  lamp", 900, 100, true);
    add_copy(&mut inv, 202, OBJECTS, "Brindle Lamp", 900, 200, true);
    // same name, other asset: two different items
    add_copy(&mut inv, 203, 1, "Kira Chair", 901, 100, true);
    add_copy(&mut inv, 204, OBJECTS, "Kira Chair", 902, 100, true);
    // no asset id given: never merged, even with the same name
    add_copy(&mut inv, 205, 1, "Zorin Rug", 0, 100, true);
    add_copy(&mut inv, 206, OBJECTS, "Zorin Rug", 0, 100, true);
    // no-copy gacha doubles
    add_copy(&mut inv, 207, 1, "Velmora Gacha Plush RARE", 903, 100, false);
    add_copy(&mut inv, 208, OBJECTS, "Velmora Gacha Plush RARE", 903, 200, false);
    let plan = duplicates_of(&inv, false);
    let listed: HashSet<Uuid> = plan.rows.iter().map(|r| r.id).collect();
    assert_eq!(
        listed,
        [id(200), id(202)].into_iter().collect::<HashSet<Uuid>>(),
        "all copies of the lamp but the oldest"
    );
    assert_eq!(plan.duplicated, 1);
    assert!(
        plan.moves(&HashSet::new()).iter().all(|m| matches!(m.to, Dest::PreTrash)),
        "to Pre-Trash, never deleted"
    );
    let plan = duplicates_of(&inv, true);
    assert!(
        plan.rows.iter().any(|r| r.id == id(208)),
        "no-copy doubles when asked, the oldest kept"
    );
    assert!(!plan.rows.iter().any(|r| r.id == id(207)));
    // a copy already in the sort folder is the one kept
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 82, 1, "Sorted");
    add_folder(&mut inv, 83, 82, "Decor");
    add_copy(&mut inv, 200, OBJECTS, "Brindle Lamp", 900, 100, true);
    add_copy(&mut inv, 201, 83, "Brindle Lamp", 900, 300, true);
    let plan = duplicates_of(&inv, false);
    let listed: Vec<Uuid> = plan.rows.iter().map(|r| r.id).collect();
    assert_eq!(listed, [id(200)], "the filed copy stays, the loose one goes");
}

/// A store's boilerplate in every delivery (its notecard, landmark,
/// unpacker, redelivery HUD) is never a duplicate: a delivery is never
/// opened. Only a second identical delivery goes, whole.
#[test]
fn store_boilerplate_inside_deliveries_is_never_a_duplicate() {
    let mut inv = fixture(&[]);
    for (n, product) in ["Velmora - Hex Dress", "Velmora - Ryu Jeans", "Velmora - Lace Top"]
        .iter()
        .enumerate()
    {
        let at = 300 + n as u128 * 10;
        add_folder(&mut inv, at, OBJECTS, product);
        add_copy(&mut inv, at + 1, at, &format!("{product} Maitreya"), 950 + n as u128, 100, true);
        // the same in every delivery
        add_copy(&mut inv, at + 2, at, "Velmora Unpacker", 990, 100, true);
        add_copy(&mut inv, at + 3, at, "Redelivery HUD", 991, 100, true);
        add_copy(&mut inv, at + 4, at, "Thank you <3", 992, 100, true);
    }
    // the very same delivery twice, the newer goes whole
    add_folder(&mut inv, 400, OBJECTS, "Velmora - Hex Dress");
    add_copy(&mut inv, 401, 400, "Velmora - Hex Dress Maitreya", 950, 500, true);
    add_copy(&mut inv, 402, 400, "Velmora Unpacker", 990, 500, true);
    add_copy(&mut inv, 403, 400, "Redelivery HUD", 991, 500, true);
    add_copy(&mut inv, 404, 400, "Thank you <3", 992, 500, true);
    let plan = duplicates_of(&inv, false);
    let listed: Vec<Uuid> = plan.rows.iter().map(|r| r.id).collect();
    assert_eq!(listed, [id(400)], "only the second identical delivery, whole");
    assert!(plan.rows[0].folder);
    assert_eq!(plan.whole_deliveries, 1);
}

/// Real use: a head's skin is a skin, never the head's add-on or makeup;
/// its makeup layers stay makeup.
#[test]
fn a_heads_skin_is_a_skin() {
    for name in [
        "{ Zorina } Ruellia Lel Evo X Tone Rose {Wear Me}",
        "[kira] mavis / ghost / no brows / evo X",
        "BRINDLE > Neremire Body+EVOX Overlay - 09",
        "Velmora Honey Lel Evo X",
    ] {
        let read = classify(name, 6, 0);
        assert_eq!(read.category, Some(Category::Skins), "{name}");
    }
    assert_eq!(
        classify("{ Zorina } Ruellia Lel Evo X Tone Blush", 6, 0).category,
        Some(Category::Makeup)
    );
    assert_eq!(
        classify("[kira] beauty glaze / highlighter pink / evo X", 6, 0).category,
        Some(Category::Makeup)
    );
    assert_eq!(classify("Kira Fangs Genus Addon", 6, 0).category, Some(Category::Heads));
}

/// Real use: a store's numbered series of deliveries is one store.
#[test]
fn a_numbered_series_is_one_store() {
    let plain = store("Brindle&Co - Tea Set").expect("store");
    for name in [
        "2. Brindle&Co",
        "16. Brindle&Co",
        "#4 Brindle&Co",
        "(3) Brindle&Co",
        "04 - Brindle&Co",
    ] {
        let found = store(name);
        assert!(found.as_deref().is_some_and(|s| same_store(s, &plain)), "{name}: {found:?}");
    }
    assert_eq!(without_number("3D Ears"), "3D Ears");
    assert_eq!(without_number("2nd Life Hat"), "2nd Life Hat");
    assert_eq!(without_number("12"), "12");
}

/// Clean up says how many items the locked default folders hold.
#[test]
fn clean_up_counts_what_the_locked_default_folders_hold() {
    let mut inv = fixture(&[]);
    inv.folders.insert(id(60), folder(id(60), id(1), "Clothing", 5));
    inv.folders.get_mut(&id(1)).expect("root").children.push(id(60));
    add_item(&mut inv, 600, 60, "Velmora Dress", 100);
    add_item(&mut inv, 601, 60, "Kira Skirt", 100);
    let none = HashSet::new();
    let plan = plan_cleanup(
        &inv,
        &Preferences::default(),
        &Cleanup::default(),
        inv.root,
        2_000_000_000,
        &none,
        &none,
    );
    assert_eq!(plan.locked_items, 2);
    assert!(!plan.rows.iter().any(|r| r.id == id(600)), "never read while locked");
}

/// Real use: a delivery whose name says nothing goes where most of its
/// things say; a mixed one stays.
#[test]
fn a_delivery_named_for_nothing_follows_its_items() {
    let none = HashSet::new();
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 900, 1, "B.RINDLE-SUGAR FUZZ-SET4");
    for (n, item) in ["Sugar Fuzz Dress Black", "Sugar Fuzz Dress Pink", "Sugar Fuzz Dress HUD"]
        .iter()
        .enumerate()
    {
        add_item(&mut inv, 950 + n as u128, 900, item, 0);
    }
    add_typed(&mut inv, 954, 900, "Read me", 0, 7);
    add_folder(&mut inv, 901, 1, "$ZR$ TAHARI");
    for (n, item) in ["Tahari Boots", "Tahari Lamp", "Tahari Hair"].iter().enumerate() {
        add_item(&mut inv, 960 + n as u128, 901, item, 0);
    }
    let plan = plan_whole(&inv, &Preferences::default(), &none, &none);
    let dress = plan.moves.iter().find(|m| m.id == id(900)).expect("follows its items");
    assert!(dress.label.contains(Category::Dresses.folder()), "{}", dress.label);
    assert!(!plan.moves.iter().any(|m| m.id == id(901)), "a mixed one stays");
}

/// Two identical deliveries of no-copy things (gacha doubles kept to
/// trade) stay unless the no-copy switch takes them.
#[test]
fn identical_no_copy_deliveries_stay_unless_asked() {
    let mut inv = fixture(&[]);
    for at in [300u128, 310] {
        add_folder(&mut inv, at, OBJECTS, "Velmora - Gacha Plush");
        add_copy(&mut inv, at + 1, at, "Velmora Gacha Plush RARE", 950, at as i64, false);
        add_copy(&mut inv, at + 2, at, "Velmora Gacha Plush Card", 951, at as i64, true);
    }
    assert!(duplicates_of(&inv, false).rows.is_empty(), "kept to trade");
    assert_eq!(duplicates_of(&inv, true).whole_deliveries, 1, "when asked, the second goes whole");
}

/// Audit 55: numbered shelves, a bare number, brows beside a head, flexi
/// and platform only alone, a lone mod tag, a sub-shelf that says nothing.
#[test]
fn audit_55_misfires() {
    assert_eq!(without_number("100 Roses"), "100 Roses");
    assert_eq!(without_number("24 Karat Gold"), "24 Karat Gold");
    assert_eq!(without_number("12. Brindle&Co"), "Brindle&Co");
    assert_eq!(without_number("04 - Brindle&Co"), "Brindle&Co");
    assert!(store_tagged("02. Casual").is_none(), "a numbered shelf is no store's delivery");
    assert_eq!(classify("Velmora Brows Honey EvoX", 6, 0).category, Some(Category::Makeup));
    assert_eq!(classify("Kira Lashes Light Lelutka", 6, 0).category, Some(Category::Makeup));
    assert_eq!(
        classify("[kira] mavis / ghost / no brows / evo X", 6, 0).category,
        Some(Category::Skins)
    );
    assert_ne!(classify("Velmora Flexi Gown", 6, 0).category, Some(Category::Hair));
    assert_eq!(classify("Velmora Flexi Bun", 6, 0).category, Some(Category::Hair));
    assert_eq!(classify("Brindle Flexi", 6, 0).category, Some(Category::Hair));
    assert_ne!(classify("Brindle Skybox Platform", 6, 0).category, Some(Category::Shoes));
    assert_eq!(classify("[Velmora] Daisie Platforms - noir", 6, 0).category, Some(Category::Shoes));
    assert_eq!(classify("Kira Platform Reborn", 6, 0).category, Some(Category::Shoes));
    assert_ne!(classify("Kira Floor Lamp (Mod)", 6, 0).category, Some(Category::BodyAddons));
    assert_eq!(classify("Kira Belly Mod", 6, 0).category, Some(Category::BodyAddons));
    // sorting one folder: a sub-shelf that says nothing stays
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 80, 1, "Clothes");
    add_folder(&mut inv, 81, 80, "Summer");
    for (n, item) in [
        "Velmora Hex Dress",
        "Kira Gown",
        "Zorin Dress",
        "Lace Dress",
        "Kira Top",
        "Velmora Tee",
    ]
    .iter()
    .enumerate()
    {
        add_item(&mut inv, 900 + n as u128, 81, item, 0);
    }
    let none = HashSet::new();
    let plan = plan(&inv, id(80), &Preferences::default(), &none, &none);
    assert!(!plan.moves.iter().any(|m| m.id == id(81)), "the user's shelf stays");
}

/// Audit 66: common names a shorter word of another kind took.
#[test]
fn hats_bean_bags_fish_tanks_and_eye_masks() {
    assert_eq!(classify("Brindle Top Hat", 6, 0).category, Some(Category::Accessories));
    assert_eq!(classify("Brindle Bean Bag Chair", 6, 0).category, Some(Category::Furniture));
    assert_eq!(classify("Kira Sleeping Bag", 6, 0).category, Some(Category::Furniture));
    assert_eq!(classify("Zorin Fish Tank", 6, 0).category, Some(Category::Decor));
    assert_eq!(classify("Kira Sleep Eye Mask", 6, 0).category, Some(Category::Accessories));
    assert_eq!(classify("Velmora Lace Top", 6, 0).category, Some(Category::Tops));
}

/// Audit 71: flexi builds are no hair.
#[test]
fn flexi_builds_are_no_hair() {
    for name in ["Brindle Flexi Grass", "Brindle Flexi Tree", "Kira Flexi Flag", "Zorin Flexi Banner"] {
        assert_eq!(classify(name, 6, 0).category, Some(Category::Decor), "{name}");
    }
    assert_eq!(classify("Velmora Flexi Bun", 6, 0).category, Some(Category::Hair));
}

/// Audit 72: costumes named for a decor thing, pins.
#[test]
fn costumes_and_pins() {
    assert_eq!(classify("Brindle Christmas Tree Costume", 6, 0).category, Some(Category::Outfits));
    assert_eq!(classify("Kira Flag Outfit", 6, 0).category, Some(Category::Outfits));
    assert_eq!(classify("Kira Flag Pin", 6, 0).category, Some(Category::Jewelry));
    assert_eq!(classify("Brindle Christmas Tree Decor", 6, 0).category, Some(Category::Decor));
    assert_eq!(classify("Brindle Tree Top Star", 6, 0).category, Some(Category::Decor));
}

/// Audit 73: a bare pin is no jewelry; bobby pins are accessories.
#[test]
fn bare_pins_are_no_jewelry() {
    assert_ne!(classify("Brindle Rolling Pin", 6, 0).category, Some(Category::Jewelry));
    assert_ne!(classify("Brindle Bowling Pins", 6, 0).category, Some(Category::Jewelry));
    assert_eq!(classify("Kira Bobby Pins", 6, 0).category, Some(Category::Accessories));
    assert_eq!(classify("Kira Enamel Pin", 6, 0).category, Some(Category::Jewelry));
}

/// Audit 78: a store's own bracketed name never decides the category.
#[test]
fn a_stores_bracketed_name_never_decides_the_category() {
    let path = |name: &str| destination(name, 6, 0, &[]).map(|p| p.join("/")).unwrap_or_default();
    assert!(
        path("[Moon Hair] Ada Dress").contains(Category::Dresses.folder()),
        "{}",
        path("[Moon Hair] Ada Dress")
    );
    assert!(
        path("[Lazy Eye] Mom Jeans").contains(Category::Pants.folder()),
        "{}",
        path("[Lazy Eye] Mom Jeans")
    );
    assert!(
        path("{Skin Deep} Bikini").contains(Category::Swimwear.folder()),
        "{}",
        path("{Skin Deep} Bikini")
    );
    // the rest says nothing: the whole name
    assert!(
        path("[Moon Hair] Ada").contains(Category::Hair.folder()),
        "{}",
        path("[Moon Hair] Ada")
    );
    // a body tag keeps its body
    assert!(
        path("[REBORN] Kitty Dress").starts_with("Reborn"),
        "{}",
        path("[REBORN] Kitty Dress")
    );
    // audit 80: a series number before the tag, a head's word in a store's tag
    assert!(
        path("01. [Moon Hair] Ada Dress").contains(Category::Dresses.folder()),
        "{}",
        path("01. [Moon Hair] Ada Dress")
    );
    // a decor or a costume store's name still decides
    assert!(
        path("[Fancy Decor] Rain Boots").contains(Category::Decor.folder()),
        "{}",
        path("[Fancy Decor] Rain Boots")
    );
    assert!(
        path("[Ada Costume] Christmas Tree").contains(Category::Outfits.folder()),
        "{}",
        path("[Ada Costume] Christmas Tree")
    );
    // a decor thing in a store's name does not decide
    assert!(
        path("[Willow Tree] Ada Poses").contains(Category::Animations.folder()),
        "{}",
        path("[Willow Tree] Ada Poses")
    );
    // what an item fits or goes on never names what it is
    for (name, category) in [
        ("[Ink Tattoo] Ada LeLutka EvoX", Category::Tattoos),
        ("[Glam Skins] Ada Catwa", Category::Skins),
        ("[Ink Tattoo] Ada Omega Applier", Category::Tattoos),
        ("[Moon Hair] Ada Addon", Category::Hair),
        ("[Gem Jewelry] Ada Ear Cuff", Category::Jewelry),
        ("[Gem Jewelry] Ada Belly Chain", Category::Jewelry),
        ("[Doll Makeup] Smoky Eyes", Category::Makeup),
        ("[Ink Tattoo] Bikini Tan Lines", Category::Tattoos),
        ("[Ink Tattoo] Henna Anklet", Category::Tattoos),
        ("[Ink Tattoo] Ada Choker", Category::Tattoos),
        ("[Moon Hair] Top Knot", Category::Hair),
        ("[Moon Hair] Ada - Henna", Category::Hair),
        ("Christmas Tree Poses", Category::Animations),
        ("Flag HUD", Category::Huds),
        ("Red Truck with Christmas Tree", Category::Decor),
        ("Lady Head Vase", Category::Decor),
    ] {
        assert!(path(name).contains(category.folder()), "{name}: {}", path(name));
    }
    // a worn layer named for a decor thing is its layer
    assert_eq!(classify("Brazil Flag Face Paint", 5, 14).category, Some(Category::Tattoos));
    // worn things named for a decor thing, as objects
    for (name, category) in [
        ("Pride Flag Cloak", Category::Jackets),
        ("Shawl Collar Sweater", Category::Tops),
        ("Cape Sleeve Top", Category::Tops),
        ("Pride Flag Sash", Category::Accessories),
        ("Flag Bandana", Category::Accessories),
        ("Christmas Tree Onesie", Category::Sleepwear),
        ("Xmas Tree Top", Category::Tops),
    ] {
        assert_eq!(classify(name, 6, 0).category, Some(category), "{name}");
    }
    assert_eq!(
        classify("Pride Flag Face Paint - LeLutka EvoX", 5, 14).category,
        Some(Category::Tattoos)
    );
    assert_eq!(classify("Pride Flag Decor", 5, 14).category, Some(Category::Decor));
    // a pack word in the rest says how it is sold, not what it is
    assert!(
        path("[Moon Hair] Ada Fatpack").contains(Category::Hair.folder()),
        "{}",
        path("[Moon Hair] Ada Fatpack")
    );
    assert!(
        path("[Swallow Jewelry] Lily Set").contains(Category::Jewelry.folder()),
        "{}",
        path("[Swallow Jewelry] Lily Set")
    );
    // a set's piece under a set's bracketed name is that set
    assert!(
        path("[Summer Bikini] Top - Black").contains(Category::Swimwear.folder()),
        "{}",
        path("[Summer Bikini] Top - Black")
    );
    assert!(
        path("[Lace Lingerie] Ada Dress").contains(Category::Dresses.folder()),
        "{}",
        path("[Lace Lingerie] Ada Dress")
    );
    assert!(
        path("[Summer Bikini] Top - Legacy").contains(Category::Swimwear.folder()),
        "{}",
        path("[Summer Bikini] Top - Legacy")
    );
    assert!(
        path("[Lace Lingerie] Top").contains(Category::Lingerie.folder()),
        "{}",
        path("[Lace Lingerie] Top")
    );
    assert!(
        path("[Summer Bikini] Top").contains(Category::Swimwear.folder()),
        "{}",
        path("[Summer Bikini] Top")
    );
    assert!(
        path("(3) [Moon Hair] Ada Dress").contains(Category::Dresses.folder()),
        "{}",
        path("(3) [Moon Hair] Ada Dress")
    );
    assert!(
        path("(2 Moons Hair) Ada Dress").contains(Category::Dresses.folder()),
        "{}",
        path("(2 Moons Hair) Ada Dress")
    );
    assert!(
        path("[Red Head] Ada HUD").contains(Category::Huds.folder()),
        "{}",
        path("[Red Head] Ada HUD")
    );
    // a tag that names the kind, a head's tag, a wearable's layer,
    // decorated marks
    assert!(
        path("[Hair] Mia Bow").contains(Category::Hair.folder()),
        "{}",
        path("[Hair] Mia Bow")
    );
    assert!(
        path("[LeLutka] Bento Addon").contains(Category::Heads.folder()),
        "{}",
        path("[LeLutka] Bento Addon")
    );
    assert_eq!(
        target("[Moon Hair] Ada", 5, 14, None, &[], false)
            .map(|d| matches!(d, Dest::Path(p) if p.iter().any(|s| s == Category::Hair.folder()))),
        Some(true),
        "a tattoo layer named for nothing keeps the whole name's reading"
    );
    assert!(
        path("**Moon Hair** Ada Dress").contains(Category::Dresses.folder()),
        "{}",
        path("**Moon Hair** Ada Dress")
    );
}

/// A demo is a name with the word demo, however it is written; never a
/// word built on it.
#[test]
fn a_demo_is_the_word_demo_never_demon() {
    for name in [
        "[Velmora] Aftermath Shirt - DEMO",
        "[M O O N] - KIRA. Sneakers DEMO",
        "(Demo) Ada Dress",
        "Ada Dress [DEMO]",
        "Ada_Hair_DEMO",
        "-DEMO- Ada Boots",
        "Velmora Demo Pack",
        "Ada Hair Demos",
        "Ada Dress DEMO.",
        "Ada Dress ...DEMO...",
    ] {
        assert!(is_demo(name), "{name}");
    }
    for name in [
        "{Mira} Demon Garters - Greyscale",
        "Demolition Truck",
        "Democrat Sign",
        "Demonia Boots",
        "Ada Dress",
        "Demolished Wall",
        "DEMO.KIRA.SALON - Ada Hair 'S'",
    ] {
        assert!(!is_demo(name), "{name}");
    }
}

fn demos_of(inv: &Inventory, worn: &HashSet<Uuid>) -> CleanupPlan {
    let cleanup = Cleanup {
        find: Find::Demos,
        ..Default::default()
    };
    plan_demos(inv, &Preferences::default(), &cleanup, inv.root, &HashSet::new(), worn)
}

/// The demo finder lists demo items, a delivered demo folder whole, and
/// demos inside a store's delivery; never a worn demo, a link, nor a name
/// built on the word.
#[test]
fn the_demo_finder_lists_demos_and_never_what_is_worn_or_a_link() {
    let mut inv = fixture(&[]);
    add_copy(&mut inv, 200, OBJECTS, "[Velmora] Aftermath Shirt - DEMO", 900, 100, true);
    add_copy(&mut inv, 201, OBJECTS, "{Mira} Demon Garters - Greyscale", 901, 100, true);
    add_copy(&mut inv, 202, 1, "[M O O N] - KIRA. Sneakers DEMO", 902, 100, true);
    // a delivered demo: the folder goes whole
    add_folder(&mut inv, 300, OBJECTS, "[Velmora] Hex Dress DEMO");
    add_copy(&mut inv, 301, 300, "Hex Dress DEMO Maitreya", 903, 100, true);
    add_copy(&mut inv, 302, 300, "Hex Dress DEMO Legacy", 904, 100, true);
    // a store's delivery holding its demo too: the demo item only
    add_folder(&mut inv, 310, OBJECTS, "[Velmora] Ryu Jeans");
    add_copy(&mut inv, 311, 310, "Ryu Jeans Maitreya", 905, 100, true);
    add_copy(&mut inv, 312, 310, "Ryu Jeans DEMO", 906, 100, true);
    // worn: stays
    add_copy(&mut inv, 320, OBJECTS, "Ada Hair DEMO", 907, 100, true);
    // a link to a demo: stays
    inv.add_items(vec![InvItem {
        id: id(330),
        parent: id(OBJECTS),
        name: "Ada Boots DEMO".into(),
        asset_type: 24,
        inv_type: 6,
        asset_id: id(320),
        owner: id(AGENT),
        ..Default::default()
    }]);
    let worn: HashSet<Uuid> = [id(320)].into_iter().collect();
    let plan = demos_of(&inv, &worn);
    let listed: HashSet<Uuid> = plan.rows.iter().map(|r| r.id).collect();
    assert_eq!(
        listed,
        [id(200), id(202), id(300), id(312)].into_iter().collect::<HashSet<Uuid>>(),
        "demo items, the demo folder whole, the demo in a delivery"
    );
    assert!(plan.rows.iter().any(|r| r.id == id(300) && r.folder && r.items == 2));
    assert!(
        plan.moves(&HashSet::new()).iter().all(|m| matches!(m.to, Dest::PreTrash)),
        "to Pre-Trash, never deleted"
    );
}

/// A store's shelf of the user's whose name says nothing (`Nookery` holding
/// `Nookery. cemetery plaque [box]`) is opened: its products are sorted
/// one by one, each moving whole, instead of the whole shelf staying as not
/// recognized. A lone product counts when the shelf is named for its store.
#[test]
fn a_whole_sort_opens_an_unnamed_store_shelf() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 80, 1, "Unsorted");
    add_folder(&mut inv, 81, 80, "Nookery");
    add_folder(&mut inv, 82, 81, "Nookery. cemetery plaque [box]");
    add_item(&mut inv, 200, 82, "Nookery. cemetery plaque", 0);
    add_folder(&mut inv, 83, 81, "Nookery. hanging candle / silver [box]");
    add_item(&mut inv, 201, 83, "Nookery. hanging candle / silver", 0);
    add_folder(&mut inv, 84, 80, "Velmira");
    add_folder(&mut inv, 85, 84, "[Velmira] Glossy Dress Black (unpacked)");
    add_item(&mut inv, 202, 85, "[Velmira] Glossy Dress Black", 0);
    let prefs = Preferences::default();
    let plan = plan_whole(&inv, &prefs, &HashSet::new(), &HashSet::new());
    let moved = |at: u128| plan.moves.iter().find(|m| m.id == id(at)).map(|m| m.label.clone());
    let base = prefs.base_name();
    assert_eq!(
        moved(82),
        Some(format!("{base} / {}", cat(Category::Decor))),
        "sources {:?}; unknown {:?}",
        whole_sources(&inv, &prefs)
            .iter()
            .map(|s| &inv.folders[s].info.name)
            .collect::<Vec<_>>(),
        plan.unknown,
    );
    assert_eq!(moved(83), Some(format!("{base} / {}", cat(Category::Decor))));
    assert_eq!(moved(85), Some(format!("{base} / {}", cat(Category::Dresses))));
    // the shelves stay, the products' items stay in their products
    for at in [81, 84, 200, 201, 202] {
        assert_eq!(moved(at), None, "{at}");
    }
    assert!(plan.unknown.is_empty(), "{:?}", plan.unknown);
}

/// One product's colours (`Aphrodite` holding `Aphrodite Black` and
/// `Aphrodite Red`) and a delivery with its parts (`HUD`, `Textures`) are
/// never opened as a shelf.
#[test]
fn a_products_colours_and_parts_are_no_shelf() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 80, 1, "Unsorted");
    add_folder(&mut inv, 81, 80, "Aphrodite");
    add_folder(&mut inv, 82, 81, "Aphrodite Black");
    add_item(&mut inv, 200, 82, "Aphrodite Black", 0);
    add_folder(&mut inv, 83, 81, "Aphrodite Red");
    add_item(&mut inv, 201, 83, "Aphrodite Red", 0);
    add_folder(&mut inv, 84, 80, "Velmora Selene");
    add_folder(&mut inv, 85, 84, "HUD");
    add_item(&mut inv, 202, 85, "Velmora Selene HUD", 0);
    add_folder(&mut inv, 86, 84, "Textures");
    add_item(&mut inv, 203, 86, "Velmora Selene Texture 1", 0);
    let reserved = sorter_folder_names(&Preferences::default());
    for shelf in [81, 84] {
        let f = &inv.folders[&id(shelf)];
        assert!(!shelf_of_products(&inv, f, f.info.name.trim(), &reserved), "{}", f.info.name);
    }
    let plan = plan_whole(&inv, &Preferences::default(), &HashSet::new(), &HashSet::new());
    for at in [82, 83, 85, 86, 200, 201, 202, 203] {
        assert!(plan.moves.iter().all(|m| m.id != id(at)), "{at}");
    }
}

#[test]
fn mixed_store_shelves_open_but_their_products_stay_together() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 80, OBJECTS, "JuniperWorks");
    add_item(&mut inv, 200, 80, "JuniperWorks * Garden Bench - White", 0);
    add_folder(&mut inv, 81, 80, "JuniperWorks * Driftwood Planter (unpacked)");
    add_item(&mut inv, 201, 81, "Planter", 0);
    add_folder(&mut inv, 82, 80, "JuniperWorks * Bay Candle [box]");
    add_item(&mut inv, 202, 82, "Candle", 0);
    let prefs = Preferences::default();
    let none = HashSet::new();
    for planned in [
        plan_whole(&inv, &prefs, &none, &none),
        plan(&inv, id(OBJECTS), &prefs, &none, &none),
        plan_entries(&inv, &[id(80)], &prefs, &none, &none),
    ] {
        assert_eq!(
            planned.moves.iter().map(|m| m.id).collect::<HashSet<_>>(),
            HashSet::from([id(81), id(82), id(200)])
        );
        assert!(planned.unknown.is_empty());
    }

    // Two variants of one product, including its demo, provide no shelf evidence.
    let names = ["JuniperWorks * Bay Candle Blue [box]", "JuniperWorks * Bay Candle Red (unpacked)"];
    for (at, name) in [(81, names[0]), (82, names[1])] {
        inv.folders.get_mut(&id(at)).expect("delivery").info.name = name.into();
    }
    let reserved = sorter_folder_names(&prefs);
    let shelf = &inv.folders[&id(80)];
    assert!(!shelf_of_products(&inv, shelf, &shelf.info.name, &reserved));
    inv.folders.get_mut(&id(81)).expect("delivery").info.name = "JuniperWorks * Cedar Tree Demo [box]".into();
    assert!(!shelf_of_products(&inv, &inv.folders[&id(80)], "JuniperWorks", &reserved));
    inv.folders.get_mut(&id(81)).expect("delivery").info.name = "JuniperWorks * Cedar Tree [box]".into();
    inv.items.get_mut(&id(200)).expect("item").name = "JuniperWorks [box]".into();
    assert!(!shelf_of_products(&inv, &inv.folders[&id(80)], "JuniperWorks", &reserved));
}

#[test]
fn a_scoped_sort_reads_a_deep_archive_and_keeps_guarded_branches_closed() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 60, OBJECTS, "Casual");
    let mut parent = 60;
    for at in 61..125 {
        add_folder(&mut inv, at, parent, &format!("Archive {at}"));
        parent = at;
    }
    add_folder(&mut inv, 125, parent, "Dresses");
    add_item(&mut inv, 200, 125, "Gold Dress", 0);
    add_folder(&mut inv, 126, 125, "[Birch Vale] Rose Dress (unpacked)");
    add_item(&mut inv, 201, 126, "Rose Dress", 0);
    add_folder(&mut inv, 127, 125, "Private archive");
    add_item(&mut inv, 202, 127, "Velvet Skirt", 0);
    add_folder(&mut inv, 128, 125, "Still loading");
    add_item(&mut inv, 204, 128, "Cedar Lamp", 0);
    inv.folders.get_mut(&id(128)).expect("shelf").state = FetchState::Unknown;
    add_item(&mut inv, 203, OBJECTS, "Jade Bracelet", 0);
    let prefs = Preferences::default();
    let protected = HashSet::from([id(127)]);
    let none = HashSet::new();
    let planned = plan(&inv, id(60), &prefs, &protected, &none);
    assert_eq!(
        planned.moves.iter().map(|m| m.id).collect::<HashSet<_>>(),
        HashSet::from([id(126), id(200)])
    );
    assert!(planned.unfetched.contains(&id(128)));
    let prefs = Preferences { folders: false, ..prefs };
    let loose_only = plan(&inv, id(60), &prefs, &protected, &none);
    assert_eq!(loose_only.moves.iter().map(|m| m.id).collect::<Vec<_>>(), [id(200)]);
    inv.folders.get_mut(&id(128)).expect("shelf").state = FetchState::Fetched;
    inv.generation += 1;
    let loaded = plan(&inv, id(60), &prefs, &protected, &none);
    assert_eq!(
        loaded.moves.iter().map(|m| m.id).collect::<HashSet<_>>(),
        HashSet::from([id(200), id(204)])
    );
}

#[test]
fn a_folder_sort_and_selected_shelf_keep_the_delivery_whole() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 80, 1, "My Archive");
    add_folder(&mut inv, 81, 80, "[Velmira]");
    add_folder(&mut inv, 82, 81, "[Velmira] Glossy Dress Black (unpacked)");
    add_item(&mut inv, 200, 82, "Glossy Dress Black", 0);
    add_folder(&mut inv, 83, 81, "[Velmira] Matte Skirt Blue (unpacked)");
    add_item(&mut inv, 201, 83, "Matte Skirt Blue", 0);
    let prefs = Preferences::default();
    let none = HashSet::new();
    for planned in [
        plan(&inv, id(80), &prefs, &none, &none),
        plan_entries(&inv, &[id(81), id(82)], &prefs, &none, &none),
    ] {
        assert_eq!(
            planned.moves.iter().map(|m| m.id).collect::<HashSet<_>>(),
            HashSet::from([id(82), id(83)])
        );
        assert!(planned.moves.iter().all(|m| m.folder));
        assert!(planned.unknown.is_empty());
    }
}

#[test]
fn a_whole_sort_opens_deep_category_shelves_but_reports_intact_unknown_deliveries() {
    let mut inv = fixture(&[]);
    let mut parent = 1;
    for (child, name) in (70..).zip(["My Stuff", "More", "Misc", "Older", "Faves", "Reborn", "Dresses"]) {
        add_folder(&mut inv, child, parent, name);
        parent = child;
    }
    add_item(&mut inv, 200, parent, "Gold Dress", 0);
    add_item(&mut inv, 201, parent, "Unknown Widget", 0);
    add_folder(&mut inv, 90, parent, "[Velmira] Vesper [box]");
    add_item(&mut inv, 202, 90, "Vesper", 0);
    add_item(&mut inv, 203, 90, "Velvet Top / Reborn", 0);
    let none = HashSet::new();
    let planned = plan_whole(&inv, &Preferences::default(), &none, &none);
    assert!(planned.moves.iter().any(|m| m.id == id(200)));
    assert!(planned.moves.iter().all(|m| ![id(90), id(202), id(203)].contains(&m.id)));
    assert!(
        planned
            .kept_list
            .iter()
            .any(|(name, reason)| name == "Velvet Top / Reborn" && *reason == Kept::Delivery)
    );
    for name in ["Unknown Widget", "[Velmira] Vesper [box]", "Vesper"] {
        assert!(planned.unknown.iter().any(|row| row == name), "{name}");
    }
}

#[test]
fn labeled_manual_shelves_resolve_opaque_products_only_with_agreeing_neighbors() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 70, 1, "!! Decor");
    add_folder(&mut inv, 71, 70, "Birch Vale");
    add_item(&mut inv, 200, 71, "Garden Statue", 0);
    add_item(&mut inv, 201, 71, "Wall Sculpture", 0);
    add_item(&mut inv, 202, 70, "Vesper", 0);
    add_item(&mut inv, 214, 70, "Garden Statue", 0);
    add_item(&mut inv, 215, 70, "Wall Sculpture", 0);
    add_item(&mut inv, 216, 71, "Opaque loose part", 0);
    add_item(&mut inv, 203, 71, "Velvet Dress", 0);
    add_typed(&mut inv, 204, 71, "Albedo", NEW, 57);
    add_folder(&mut inv, 72, 71, "[Birch Vale] Sylvan [boxed]");
    add_item(&mut inv, 205, 72, "Sylvan", 0);
    add_typed(&mut inv, 206, 72, "Read me", NEW, 7);
    add_folder(&mut inv, 73, 71, "[Birch Vale] Rowan [boxed]");
    add_item(&mut inv, 207, 73, "Rowan One", 0);
    add_item(&mut inv, 208, 73, "Rowan Two", 0);
    add_item(&mut inv, 209, 73, "Velvet Jacket", 0);
    add_folder(&mut inv, 74, 70, "Mixed gifts");
    add_item(&mut inv, 210, 74, "Golden Necklace", 0);
    add_item(&mut inv, 211, 74, "Short Skirt", 0);
    add_item(&mut inv, 212, 74, "Lumen", 0);
    add_folder(&mut inv, 75, 71, "Locked");
    add_item(&mut inv, 213, 75, "Locked Vesper", 0);
    let prefs = Preferences::default();
    let protected = HashSet::from([id(75)]);
    let none = HashSet::new();
    for planned in [
        plan_whole(&inv, &prefs, &protected, &none),
        plan(&inv, id(70), &prefs, &protected, &none),
    ] {
        for (entry, category) in [
            (202, Category::Decor),
            (203, Category::Dresses),
            (204, Category::Materials),
            (72, Category::Decor),
        ] {
            assert!(
                planned
                    .moves
                    .iter()
                    .any(|m| m.id == id(entry) && m.to == Dest::Path(vec![category_folder(category)])),
                "{entry}: {:?}",
                planned.unknown
            );
        }
        assert!(planned.moves.iter().all(|m| {
            ![73, 205, 206, 207, 208, 209, 212, 213, 216]
                .into_iter()
                .any(|entry| m.id == id(entry))
        }));
        assert!(planned.unknown.iter().any(|name| name == "Lumen"));
    }
}

#[test]
fn my_rules_find_deep_sorted_matches_and_open_deliveries_only_when_enabled() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 70, 1, "!!!!!SORTED!!!!!!");
    add_folder(&mut inv, 71, 70, "Decor");
    add_folder(&mut inv, 72, 71, "My archive");
    add_folder(&mut inv, 73, 72, "Winter");
    add_folder(&mut inv, 74, 73, "[Moss Vale] Velvet Sofa FATPACK");
    add_typed(&mut inv, 200, 73, "read me", NEW, 7);
    add_typed(&mut inv, 201, 74, "read me", NEW, 7);
    add_item(&mut inv, 202, 74, "Velvet Sofa Black", NEW);
    add_typed(&mut inv, 203, 74, "read me", NEW, 7);
    let none = HashSet::new();
    let worn = HashSet::from([id(203)]);
    let mut prefs = Preferences {
        rules: vec![Rule {
            words: "read me".into(),
            folder: "Instructions".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    for mode in [Match::All, Match::Any, Match::Phrase] {
        prefs.rules[0].mode = mode;
        prefs.rules[0].inside_deliveries = false;
        let plan = plan_whole(&inv, &prefs, &none, &worn);
        assert_eq!(plan.moves.iter().map(|m| m.id).collect::<Vec<_>>(), [id(200)], "{mode:?}");
        assert!(plan.kept_list.contains(&("read me".into(), Kept::Delivery)));
        prefs.rules[0].inside_deliveries = true;
        let plan = plan_whole(&inv, &prefs, &none, &worn);
        assert_eq!(
            plan.moves.iter().map(|m| m.id).collect::<HashSet<_>>(),
            HashSet::from([id(200), id(201)])
        );
    }
    let scoped = plan(&inv, id(71), &prefs, &none, &worn);
    assert_eq!(
        scoped.moves.iter().map(|m| m.id).collect::<HashSet<_>>(),
        HashSet::from([id(200), id(201)])
    );
    prefs.rules.push(Rule {
        words: "read me".into(),
        mode: Match::Phrase,
        pre_trash: true,
        inside_deliveries: true,
        ..Default::default()
    });
    assert!(plan(&inv, id(OBJECTS), &prefs, &none, &worn).moves.is_empty());
    prefs.rules.pop();
    add_folder(&mut inv, 75, 73, "[Moss Vale] Tulip Chair [Unpack Me]");
    add_typed(&mut inv, 204, 75, "read me", NEW, 7);
    prefs.rules[0].mode = Match::Any;
    let planned = plan_whole(&inv, &prefs, &none, &worn);
    assert!(planned.moves.iter().any(|m| m.id == id(204)));
    assert!(planned.moves.iter().all(|m| m.id != id(75)));
    prefs.rules[0].skip_sorted = true;
    assert!(plan_whole(&inv, &prefs, &none, &worn).moves.is_empty());
}

#[test]
fn empty_folders_only_rechecks_the_tree_before_moving() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 70, 1, "Empty archive");
    add_folder(&mut inv, 71, 70, "Nested");
    add_folder(&mut inv, 72, 1, "Occupied archive");
    add_item(&mut inv, 200, 72, "Old chair", OLD);
    add_folder(&mut inv, 73, 1, "Unread archive");
    inv.folders.get_mut(&id(73)).expect("folder").state = FetchState::Unknown;
    let prefs = Preferences::default();
    let cleanup = Cleanup {
        find: Find::EmptyFolders,
        ..before_2020()
    };
    let none = HashSet::new();
    let plan = plan_cleanup(&inv, &prefs, &cleanup, id(1), today(), &none, &none);
    assert_eq!(cleaned(&plan), [id(70)]);
    assert!(plan.unfetched.contains(&id(73)));
    let mut queue = VecDeque::from(plan.moves(&none));
    add_item(&mut inv, 201, 71, "New chair", NEW);
    let mut skipped = 0;
    assert!(matches!(
        step(&mut inv, "Sorted", &mut queue, &none, &none, &mut skipped),
        Step::Done
    ));
    assert_eq!(skipped, 1);
}

#[test]
fn ambiguous_body_aliases_need_fit_context() {
    for (name, expected) in [
        ("[Harbor Gems] Venus Necklace", "Jewelry"),
        ("Moss Vale / Venus / Diamond Stud", "Jewelry"),
        ("MoonCove - Venus Flytrap Gang", "Decor"),
        ("[Moss Brow] Venus Stockings", "Stockings & socks"),
        ("Foldworks// Venus T-shirt", "Tops"),
        ("Pearl Boots / Venus", "Belleza/Shoes"),
        ("Velvet Skirt [Freya]", "Belleza/Skirts"),
        ("Top fits Isis", "Belleza/Tops"),
    ] {
        assert_eq!(path(name).as_deref(), Some(expected), "{name}");
    }
    assert!(!path("AsterWorks - Venus (Full Pack)").is_some_and(|p| p.contains("Belleza")));
}

#[test]
fn store_prefixes_use_a_confirmed_label_from_the_same_creator() {
    let mut inv = fixture(&["MoonCove - Accent Light", "MoonCove DJ Stage - Adult", "MoonCove DJ Stage - PG"]);
    for at in [100, 101] {
        inv.items.get_mut(&id(at)).expect("item").creator = id(900);
    }
    inv.items.get_mut(&id(102)).expect("item").creator = id(901);
    let prefs = Preferences {
        stores: true,
        ..Default::default()
    };
    let none = HashSet::new();
    let plan = plan_whole(&inv, &prefs, &none, &none);
    let label = |at| &plan.moves.iter().find(|m| m.id == id(at)).expect("item moved").label;
    assert!(label(101).ends_with(" / MoonCove"), "{}", label(101));
    assert!(!label(102).ends_with(" / MoonCove"), "{}", label(102));
}

#[test]
fn clothing_deliveries_without_a_category_use_the_consensus_across_fits() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 70, OBJECTS, "[Moss Vale] Starfall [unpack me]");
    add_item(&mut inv, 201, 70, "Starfall Kini / Blue / Reborn", NEW);
    add_item(&mut inv, 202, 70, "Starfall Kini / Blue / Hourglass", NEW);
    let prefs = Preferences {
        stores: true,
        ..Default::default()
    };
    let none = HashSet::new();
    let plan = plan_whole(&inv, &prefs, &none, &none);
    assert_eq!(plan.moves.len(), 1);
    let moved = &plan.moves[0];
    assert_eq!(moved.id, id(70));
    assert!(moved.label.contains("Several bodies / Swimwear"), "{}", moved.label);
    assert!(plan.unknown.is_empty());
}

#[test]
fn mixed_fit_consensus_never_files_a_shop_by_its_demos() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 70, OBJECTS, "MoonCove");
    add_item(&mut inv, 200, 70, "MoonCove - Velvet Top [FATPACK]", NEW);
    add_folder(&mut inv, 71, 70, "MoonCove - Velvet Top [DEMO]");
    add_item(&mut inv, 201, 71, "MoonCove - Velvet Top / Reborn [DEMO]", NEW);
    add_item(&mut inv, 202, 71, "MoonCove - Velvet Top / Hourglass [DEMO]", NEW);
    assert_eq!(target_from_items(&inv, id(70), &Preferences::default(), None, false), None);
}

#[test]
fn numbered_same_product_style_folders_use_the_named_primary_object() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 70, OBJECTS, "[Nookery] Midnight Bloom [addme] (unpacked)");
    for (variant, folder_id) in (1..=3).zip(71..=73) {
        add_folder(
            &mut inv,
            folder_id,
            70,
            &format!("[Nookery] Midnight Bloom - Style {variant} (unpacked)"),
        );
        let stem = if variant == 3 { "Midnight Bloo" } else { "Midnight Bloom" };
        add_item(
            &mut inv,
            201 + variant,
            folder_id,
            &format!("[Nookery] {stem} - Horn Style {variant}"),
            0,
        );
        for layer in 1..=4 {
            add_typed(
                &mut inv,
                300 + variant * 10 + layer,
                folder_id,
                &format!("[Nookery] Midnight Bloom - Color {layer}"),
                NEW,
                5,
            );
            inv.items.get_mut(&id(300 + variant * 10 + layer)).expect("layer").flags = 14;
        }
    }
    let none = HashSet::new();
    let prefs = Preferences::default();
    for planned in [
        plan_whole(&inv, &prefs, &none, &none),
        plan(&inv, id(OBJECTS), &prefs, &none, &none),
    ] {
        assert_eq!(planned.moves.len(), 1);
        assert_eq!(planned.moves[0].id, id(70));
        assert_eq!(planned.moves[0].to, Dest::Path(vec![category_folder(Category::Accessories)]));
    }
    inv.items.get_mut(&id(203)).expect("style 2").name = "[Nookery] Midnight Bloom - Chair Style 2".into();
    assert_eq!(numbered_product_category(&inv, &inv.folders[&id(70)]), None);
}

#[test]
fn modular_floor_packs_use_matching_physical_tiles_and_keep_variants_together() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 70, OBJECTS, "Nookery Star Floor [Black & White Pack]");
    add_item(&mut inv, 201, 70, "Nookery Star Floor//1x1 [White]", 0);
    add_item(&mut inv, 202, 70, "Nookery Star Floor//2x1 [Black/PBR]", 0);
    add_item(&mut inv, 203, 70, "Nookery Star Floor//1x1 [White/PBR] 1", 0);
    add_typed(&mut inv, 204, 70, "Nookery Floor assembly notes", 0, 7);
    let none = HashSet::new();
    let prefs = Preferences::default();
    for planned in [
        plan_whole(&inv, &prefs, &none, &none),
        plan(&inv, id(OBJECTS), &prefs, &none, &none),
    ] {
        assert_eq!(planned.moves.len(), 1);
        assert_eq!(planned.moves[0].id, id(70));
        assert_eq!(planned.moves[0].to, Dest::Path(vec![category_folder(Category::Decor)]));
    }

    add_item(&mut inv, 205, 70, "Nookery Star Floor 1024x1024 [White]", 0);
    assert!(plan_whole(&inv, &prefs, &none, &none).moves.is_empty());
}

#[test]
fn body_addon_kits_follow_matching_mesh_parts_and_keep_the_layers() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 70, OBJECTS, "Nookery Clearwater Fins Kit");
    add_item(&mut inv, 201, 70, "Nookery Clearwater Body Fins [Reborn]", 0);
    add_item(&mut inv, 202, 70, "Nookery Clearwater Hand Fins", 0);
    add_item(&mut inv, 203, 70, "Nookery Clearwater Fin Texture HUD", 0);
    // Old exports represent the instructions and layers as generic objects.
    add_item(&mut inv, 204, 70, "! About Clearwater Fin Kit", 0);
    for layer in 205..=208 {
        add_item(&mut inv, layer, 70, &format!("Nookery Clearwater Fin BOM {layer}"), 0);
    }
    let none = HashSet::new();
    let prefs = Preferences::default();
    for planned in [
        plan_whole(&inv, &prefs, &none, &none),
        plan(&inv, id(OBJECTS), &prefs, &none, &none),
    ] {
        assert_eq!(planned.moves.len(), 1);
        assert_eq!(planned.moves[0].id, id(70));
        assert_eq!(planned.moves[0].to, Dest::Path(vec![category_folder(Category::BodyAddons)]));
    }

    add_item(&mut inv, 209, 70, "Nookery Clearwater Evening Dress", 0);
    let planned = plan_whole(&inv, &prefs, &none, &none);
    assert!(planned.moves.is_empty());
    assert!(planned.unknown.iter().any(|name| name == "Nookery Clearwater Fins Kit"));
}

#[test]
fn a_numbered_flat_delivery_needs_one_consistent_named_product() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 70, OBJECTS, "Nookery Midnight Bloom [ADDME]");
    add_item(&mut inv, 201, 70, "Nookery Midnight Bloom [Horn 1]", 0);
    add_item(&mut inv, 202, 70, "Nookery Midnight Bloom [Horn 2]", 0);
    let root = &inv.folders[&id(70)];
    assert_eq!(numbered_product_category(&inv, root), Some(Category::Accessories));
    add_item(&mut inv, 203, 70, "Unrelated horn", 0);
    assert_eq!(numbered_product_category(&inv, &inv.folders[&id(70)]), None);
}

#[test]
fn a_mixed_treasure_chest_shelf_is_opened_as_an_archive() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 70, OBJECTS, "Hunt Treasure Chest");
    add_item(&mut inv, 201, 70, "Blue Velvet Dress", 0);
    add_item(&mut inv, 202, 70, "Amber Necklace", 0);
    let none = HashSet::new();
    let planned = plan_whole(&inv, &Preferences::default(), &none, &none);
    assert_eq!(
        planned.moves.iter().map(|entry| entry.id).collect::<HashSet<_>>(),
        HashSet::from([id(201), id(202)])
    );
}

#[test]
fn generic_set_names_take_a_consistent_payload_category_and_keep_the_store() {
    let mut inv = fixture(&[]);
    for (at, title) in [
        (70, "Nightfall Set"),
        (71, "Nightfall Outfit"),
        (72, "Winter Bundle"),
        (73, "Meadow Collection"),
        (74, "Summer Set"),
    ] {
        add_folder(&mut inv, at, OBJECTS, &format!("[Birch Vale] {title} [boxed]"));
    }
    for (at, parent, title) in [
        (201, 70, "Velvet Dress Blue"),
        (202, 70, "Velvet Dress Red"),
        (203, 71, "Velvet Dress Blue"),
        (204, 71, "Velvet Dress Red"),
        (205, 72, "Winter Boots Blue"),
        (206, 72, "Winter Boots Red"),
        (207, 72, "Winter Boots Black"),
        (208, 72, "Velvet Dress Red"),
    ] {
        add_item(&mut inv, at, parent, title, 0);
    }
    add_typed(&mut inv, 209, 73, "East", NEW, 0);
    add_typed(&mut inv, 210, 73, "West", NEW, 0);
    add_typed(&mut inv, 211, 73, "Read me", NEW, 7);
    add_item(&mut inv, 212, 74, "Colour Change HUD", 0);
    add_folder(&mut inv, 75, OBJECTS, "Backdrop stage stuff");
    add_item(&mut inv, 213, 75, "Silk Dress", 0);
    add_folder(&mut inv, 76, 1, "Hawker Car Red Box Free Stuff");
    add_item(&mut inv, 214, 76, "Hawker Car Red", 0);
    add_item(&mut inv, 215, 76, "Add to wear", 0);
    let prefs = Preferences {
        stores: true,
        ..Preferences::default()
    };
    let none = HashSet::new();
    let whole = plan_whole(&inv, &prefs, &none, &none);
    assert!(whole.moves.iter().any(|entry| entry.id == id(76) && entry.folder));
    for planned in [
        plan_whole(&inv, &prefs, &none, &none),
        plan(&inv, id(OBJECTS), &prefs, &none, &none),
    ] {
        assert_eq!(planned.moves.iter().filter(|entry| entry.id != id(76)).count(), 6);
        assert!(planned.moves.iter().any(|entry| entry.id == id(213) && !entry.folder));
        assert!(planned.moves.iter().all(|entry| entry.id != id(75)));
        for (at, category) in [
            (70, Category::Dresses),
            (71, Category::Outfits),
            (72, Category::Outfits),
            (73, Category::Textures),
            (74, Category::Outfits),
        ] {
            let planned_move = planned.moves.iter().find(|entry| entry.id == id(at)).expect("intact delivery");
            assert_eq!(
                planned_move.to,
                Dest::Path(vec![category_folder(category), "Birch Vale".to_owned()])
            );
        }
    }
}

#[test]
fn opaque_delivery_variants_use_an_exact_sibling_family_with_matching_contents() {
    let mut inv = fixture(&[]);
    for (at, title) in [
        (70, "Nookery {Night Bloom} Horns [boxed]"),
        (71, "Nookery {Night Bloom} Fades [boxed]"),
        (72, "Other {Night Bloom} Fades [boxed]"),
        (73, "Nookery {Night Bloom} Tips [boxed]"),
        (74, "Nookery {Night Bloom} Colourways [boxed]"),
    ] {
        add_folder(&mut inv, at, OBJECTS, title);
    }
    for (at, parent, title) in [
        (201, 70, "Nookery {Night Bloom} Horns Black"),
        (202, 71, "Nookery {Night Bloom} Fade Orange"),
        (203, 71, "Nookery {Night Bloom} Fade Red"),
        (204, 72, "Other {Night Bloom} Fade Orange"),
        (205, 73, "Something unrelated"),
        (206, 74, "Nookery {Night Bloom} Orange"),
        (207, 74, "Nookery {Night Bloom} Scarlet"),
    ] {
        add_item(&mut inv, at, parent, title, 0);
    }
    for (at, creator) in [(201, 900), (202, 900), (203, 900), (206, 901), (207, 901)] {
        inv.items.get_mut(&id(at)).expect("item").creator = id(creator);
    }
    let prefs = Preferences::default();
    let none = HashSet::new();
    for planned in [
        plan_whole(&inv, &prefs, &none, &none),
        plan(&inv, id(OBJECTS), &prefs, &none, &none),
    ] {
        let moved = planned
            .moves
            .iter()
            .find(|entry| entry.id == id(71))
            .expect("the intact variant pack");
        assert_eq!(moved.to, Dest::Path(vec![category_folder(Category::Accessories)]));
        assert!(
            planned
                .moves
                .iter()
                .all(|entry| ![72, 73, 74, 202, 203].into_iter().any(|at| entry.id == id(at)))
        );
    }
}

#[test]
fn a_sorted_family_can_confirm_the_same_manual_category_without_reading_locked_anchors() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 50, 1, "!!!!!SORTED!!!!!!");
    add_folder(&mut inv, 51, 50, "Accessories");
    add_folder(&mut inv, 52, OBJECTS, "Accessories");
    add_folder(&mut inv, 53, OBJECTS, "Old assorted stock");
    add_folder(&mut inv, 70, 51, "Nookery {Night Bloom} Horns [boxed]");
    add_folder(&mut inv, 71, 52, "Nookery {Night Bloom} Fades [boxed]");
    add_folder(&mut inv, 72, 53, "Nookery {Night Bloom} Tips [boxed]");
    add_item(&mut inv, 201, 70, "Nookery {Night Bloom} Horns", 0);
    add_item(&mut inv, 202, 71, "Nookery {Night Bloom} Fade Red", 0);
    add_item(&mut inv, 203, 72, "Nookery {Night Bloom} Tip Orange", 0);
    let prefs = Preferences {
        base: "!!!!!SORTED!!!!!!".into(),
        ..Preferences::default()
    };
    let none = HashSet::new();
    let planned = plan_whole(&inv, &prefs, &none, &none);
    assert_eq!(planned.moves.iter().map(|entry| entry.id).collect::<Vec<_>>(), [id(71)]);
    let locked = HashSet::from([id(51)]);
    let planned = plan_whole(&inv, &prefs, &locked, &none);
    assert!(planned.moves.is_empty());
    assert!(planned.unknown.contains(&"Nookery {Night Bloom} Fades [boxed]".into()));
    inv.folders.get_mut(&id(51)).expect("sorted category").state = FetchState::Failed;
    let planned = plan_whole(&inv, &prefs, &none, &none);
    assert!(planned.moves.is_empty());
}

#[test]
fn opaque_asset_packs_use_consistent_native_types_and_keep_their_contents() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 70, OBJECTS, "Meadow 1024 [boxed]");
    add_typed(&mut inv, 201, 70, "Meadow 1024 8-2", NEW, 0);
    add_typed(&mut inv, 202, 70, "Meadow 1024 1-3", NEW, 19);
    add_typed(&mut inv, 203, 70, "Read me", NEW, 7);
    add_typed(&mut inv, 204, 70, "Update checker", NEW, 10);
    let prefs = Preferences::default();
    let none = HashSet::new();
    for planned in [
        plan_whole(&inv, &prefs, &none, &none),
        plan(&inv, id(OBJECTS), &prefs, &none, &none),
    ] {
        assert_eq!(planned.moves.len(), 1);
        assert_eq!(planned.moves[0].id, id(70));
        assert_eq!(planned.moves[0].to, Dest::Path(vec![category_folder(Category::Textures)]));
    }
    add_typed(&mut inv, 205, 70, "Meadow sky", NEW, 56);
    assert_eq!(target_from_items(&inv, id(70), &prefs, None, false), None);
    inv.items.get_mut(&id(205)).expect("item").asset_type = 6;
    assert_eq!(target_from_items(&inv, id(70), &prefs, None, false), None);
}

#[test]
fn opaque_script_deliveries_use_native_payload_and_keep_the_guide() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 70, OBJECTS, "Kestrel [boxed]");
    add_typed(&mut inv, 201, 70, "Core", NEW, 10);
    add_typed(&mut inv, 202, 70, "Controller", NEW, 11);
    add_typed(&mut inv, 203, 70, "Read me", NEW, 7);
    add_typed(&mut inv, 204, 70, "Support", NEW, 3);
    let prefs = Preferences::default();
    let none = HashSet::new();
    let planned = plan_whole(&inv, &prefs, &none, &none);
    assert_eq!(planned.moves.len(), 1);
    assert_eq!(planned.moves[0].id, id(70));
    assert_eq!(planned.moves[0].to, Dest::Path(vec![category_folder(Category::Scripts)]));
    add_item(&mut inv, 205, 70, "Something unrelated", 0);
    assert_eq!(target_from_items(&inv, id(70), &prefs, None, false), None);
}

#[test]
fn sorter_uses_wearable_types_without_guessing_from_opaque_product_titles() {
    for flags in [13, 15, 16] {
        assert_eq!(classify("Untitled layer", 5, flags).category, Some(Category::BodyAddons));
    }
    assert!(classify("Friendly Fangs", 6, 0).category.is_none());
    assert_eq!(classify("Velvet Vampire Fangs", 6, 0).category, Some(Category::BodyAddons));
}

#[test]
fn sorter_uses_native_types_for_environment_presets_and_materials() {
    for (kind, category) in [(56, Category::Environment), (57, Category::Materials)] {
        assert_eq!(
            destination("Lara X Velvet Dress Squish", kind, 0, &[]),
            Some(vec![category_folder(category)])
        );
    }
    assert_eq!(
        destination("Twilight Sky Settings", -1, 0, &[]),
        Some(vec![category_folder(Category::Environment)])
    );
}

#[test]
fn sorter_uses_creator_hints_to_explain_what_stays_inside_deliveries() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 90, OBJECTS, "[Moss Vale] Vesper [box]");
    add_item(&mut inv, 200, 90, "Vesper", 0);
    add_item(&mut inv, 201, 90, "Glimmer", 0);
    add_item(&mut inv, 202, 90, "Beacon", 0);
    for (at, creator) in [(200, 801), (201, 802), (202, 802)] {
        inv.items.get_mut(&id(at)).expect("item").creator = id(creator);
    }
    let prefs = Preferences {
        store_hints: vec![StoreHint {
            category: Category::Decor,
            creator: Some(id(801)),
            ..Default::default()
        }],
        ..Default::default()
    };
    let none = HashSet::new();
    let planned = plan_whole(&inv, &prefs, &none, &none);
    assert!(planned.moves.is_empty(), "the package remains intact");
    assert!(
        planned
            .kept_list
            .iter()
            .any(|(name, why)| name == "Vesper" && *why == Kept::Delivery)
    );
    assert!(planned.unknown.iter().any(|name| name == "Glimmer"));
}

#[test]
fn the_remembered_sort_folder_keeps_its_identity_after_a_rename() {
    let mut inv = fixture(&[]);
    add_folder(&mut inv, 80, 1, "!!!!!SORTED!!!!!!");
    let mut prefs = Preferences::default();
    assert_eq!(base_folder(&inv, &prefs), Some(id(80)), "recognize the single decorated default");
    prefs.base = "!!!!!SORTED!!!!!!".into();
    assert_eq!(prefs.base_name(), "!!!!!SORTED!!!!!!");
    assert_eq!(base_folder(&inv, &prefs), Some(id(80)));
    prefs.select_account_base(&inv, id(AGENT));
    prefs.base.clear();
    let mut prefs: Preferences = serde_json::from_str(&serde_json::to_string(&prefs).expect("serialize")).expect("deserialize");

    add_folder(&mut inv, 79, 1, "My renamed destination");
    add_folder(&mut inv, 81, 1, "Sorted");
    inv.folders.get_mut(&id(80)).expect("destination").info.name = "My renamed destination".into();
    add_item(&mut inv, 200, 1, "Gold Dress", 0);
    prefs.select_account_base(&inv, id(42));
    assert_eq!(prefs.base_id, Some(id(81)), "another account falls back to the default");
    prefs.select_account_base(&inv, id(AGENT));
    let none = HashSet::new();
    let mut queue = VecDeque::from(plan_whole(&inv, &prefs, &none, &none).moves);
    assert_eq!(base_folder(&inv, &prefs), Some(id(80)));
    let base = SortBase {
        id: prefs.base_id,
        name: &prefs.base_name(),
    };
    let Step::Change(change, moves) = step_with(&mut inv, base, &mut queue, &none, &none, &mut 0, &|_, _| false) else {
        panic!("the item should be sorted");
    };
    assert!(
        change
            .operations
            .iter()
            .any(|op| matches!(op, Operation::Folder { parent, .. } if *parent == id(80)))
    );
    actions::demo_mutate(&mut inv, id(AGENT), *change).expect("demo mutation");
    assert!(moves.iter().any(|m| m.id == id(200) && arrived(&inv, base, m)));

    inv.folders.get_mut(&id(80)).expect("destination").info.parent = id(79);
    assert_eq!(
        base_folder(&inv, &prefs),
        Some(id(81)),
        "a destination moved off the root falls back to its name"
    );
}
