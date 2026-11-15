//! `AURORA_DEMO_INVENTORY=sorter|aurora-sorter`: a large, messy inventory
//! for Auto-sort, built like the ones residents collect over years. Store
//! tags, body tags and products are combined the way creators write them
//! (`.VX. Vixen Dress - Reborn Pack [Unpacker]`, `eBODY - REBORN - Wear to
//! unpack`, `FemAvg_Dress_Cut_Long_Reborn_F`): the names are the grid's
//! content the sorter reads, not interface text, so they stay in English.
//! Every store name is made up.

use super::super::{FetchState, Folder, Inventory};
use aurora_net::inventory::{InvFolder, InvItem};
use uuid::Uuid;

/// The demo's Objects folder, the one Auto-sort opens on.
pub const OBJECTS: Uuid = Uuid::from_u128(8600);
pub const CLOTHING: Uuid = Uuid::from_u128(8601);
const SORTED: Uuid = Uuid::from_u128(8602);
const SORTED_REBORN: Uuid = Uuid::from_u128(8603);

const STORES: &[&str] = &[
    ".VX.",
    "-Velvet Lies-",
    "ZETA.mods -",
    "{Moonrise}",
    "18. Nyx -",
    "*Opal&Ash*",
    "KIRA. //",
    "~Hollow Pine~",
    "Lumen -",
    "[Saint Noir]",
    "rvn -",
    ". o p a l i n e . //",
    "[Birch Hollow] +",
    "*Neverlace*",
];

const PRODUCTS: &[&str] = &[
    "Akira Apron",
    "Vixen Dress",
    "Angel Set",
    "Lace Stockings",
    "Mother of Dragons Tattoo",
    "Cosette Corset",
    "Cosette Robe",
    "Kristina Shirt",
    "Coffee Leg Garter",
    "Soft Thighs V4.2",
    "Black Ribbon Garter",
    "Denim Skirt",
    "Pleated Mini Skirt",
    "Cargo Pants",
    "Ipanema Bikini",
    "Alyssa Lingerie Set",
    "Chunky Knit Sweater",
    "Biker Jacket",
    "Platform Boots",
    "Stiletto Heels",
    "Pearl Choker",
    "Hoop Earrings",
    "Latex Catsuit",
    "my body reflection",
    "Waifu Boobs",
    "Juicy Rolls",
    "Freckles",
    "Duo toned stockings",
    "Silk Kimono",
    "Crop Top",
    "Hot Shorts",
    "Velvet Gown",
    "Fishnet Tights",
    "Sport Bra",
    "Leather Gloves",
    "Belly Chain Piercing",
    "Lip Gloss",
    "Wool Coat",
    "Tank Top",
    "Mesh Bodysuit",
];

const BODIES: &[&str] = &[
    "Reborn",
    "[REBORN]",
    "(Reborn)",
    "eBODY Reborn",
    "REBORN+Juicy",
    "Legacy F",
    "LEGACY",
    "Legacy Perky",
    "Legacy M",
    "Maitreya Lara X",
    "Lara Petite",
    "Belleza Freya",
    "Kupra",
    "Gianni",
    "eBODY Curvy",
    "",
];

const ENDINGS: &[&str] = &[
    "",
    " - Wear to unpack",
    " [Unpacker]",
    " (packed)",
    " RARE",
    " Black",
    " (add me)",
    " Pack [Unpacker]",
    " FATPACK",
    " #3",
    " sq",
    " (Squish)",
    " - Squeeze",
];

/// Things with no body, and names the sorter cannot read.
const LOOSE: &[(&str, i32)] = &[
    ("Black leather chair", 6),
    ("[Birch Hollow] + Tucker Round Gothic Table (regular pack)", 6),
    ("[Birch Hollow] + Semaj Pillow Couch (black)", 6),
    ("*Neverlace* Neon Heart Shelf - Light Pink", 6),
    // a texture whose name says chair: its type wins
    ("Bar bottom texture (without chair shadows)", 0),
    ("Oak dining table", 6),
    ("Velvet sofa - Rose", 6),
    ("Fern plant in clay pot", 6),
    ("Brass floor lamp", 6),
    ("Cozy rug - grey", 6),
    ("Gift box", 6),
    ("Mystery Crate", 6),
    ("HUD v2.1", 6),
    ("Group join board", 6),
    ("Lumen - store", 3),
    ("Read me first", 7),
    ("Welcome to {Moonrise}", 7),
    ("Wave hello", 20),
];

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

fn item(id: u128, parent: Uuid, name: String, asset_type: i32, flags: u32, agent: Uuid) -> InvItem {
    use super::super::actions::{COPY, MODIFY, MOVE, TRANSFER};
    // no copy and no modify for most, as the grid's products usually are
    let owner_mask = if id.is_multiple_of(3) {
        COPY | MOVE
    } else {
        MODIFY | TRANSFER | MOVE
    };
    InvItem {
        id: Uuid::from_u128(id),
        parent,
        name,
        asset_type,
        inv_type: match asset_type {
            5 | 13 => 18,
            20 => 19,
            other => other,
        },
        flags,
        owner: agent,
        creator: Uuid::from_u128(8700 + id % 12),
        base_mask: owner_mask,
        owner_mask,
        next_owner_mask: owner_mask,
        created_at: acquired(id),
        ..Default::default()
    }
}

/// 1 January 2008, 00:00 UTC.
const Y2008: i64 = 1_199_145_600;
/// About a year, in seconds.
const YEAR: i64 = 31_556_952;

/// An acquired date between 2008 and the end of 2026, the same on every run,
/// for Clean up's chips and custom dates.
fn acquired(id: u128) -> i64 {
    let spread = (id.wrapping_mul(2_654_435_761) % (19 * YEAR as u128)) as i64;
    Y2008 + spread
}

/// Seed the folders and about 700 entries. `scale` multiplies the Objects
/// folder for a heavier run (`sorter-big`).
pub fn seed(inv: &mut Inventory, agent: Uuid, scale: usize) {
    let root = inv.root;
    for (id, parent, name, kind) in [
        (OBJECTS, root, aurora_i18n::tr!("common-objects"), 6),
        (CLOTHING, root, aurora_i18n::tr!("common-clothing"), 5),
        // a sorted folder from an earlier run, reused rather than doubled
        (SORTED, root, aurora_i18n::tr!("inventory-sorter-default-base"), -1),
        (SORTED_REBORN, SORTED, "Reborn", -1),
    ] {
        inv.folders.insert(id, folder(id, parent, name, kind));
        if let Some(f) = inv.folders.get_mut(&parent)
            && !f.children.contains(&id)
        {
            f.children.push(id);
        }
    }
    let mut items = Vec::new();
    let mut next = 8_610_000u128;
    // a fixed walk through the tables: the same inventory on every run
    let mut seed = 0x2545_f491u64;
    let mut pick = |len: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % len as u64) as usize
    };
    for _ in 0..600 * scale.max(1) {
        let store = STORES[pick(STORES.len())];
        let product = PRODUCTS[pick(PRODUCTS.len())];
        let body = BODIES[pick(BODIES.len())];
        let ending = ENDINGS[pick(ENDINGS.len())];
        let name = match pick(4) {
            0 => format!("{body} - {product}{ending}"),
            1 => format!("{store} {product} {body}{ending}"),
            2 => format!("{}_{}{}", product.replace(' ', "_"), body.replace(' ', "_"), ending),
            _ => format!("{store} {product} - {body}{ending}"),
        };
        items.push(item(next, OBJECTS, name.trim().replace("  ", " "), 6, 0, agent));
        next += 1;
    }
    for (name, kind) in LOOSE {
        items.push(item(next, OBJECTS, (*name).into(), *kind, 0, agent));
        next += 1;
    }
    // System clothing layers: the wearable type in the flags names the
    // category when the name does not (tattoo 14, skirt 12, socks 7).
    for (name, kind, flags) in [
        ("Mother of Dragons BoM - Reborn", 5, 14),
        ("Nyx layer", 5, 12),
        ("Opal socks layer", 5, 7),
        ("Default shape", 13, 0),
        ("Abyss eyes", 13, 3),
        ("Legacy F alpha", 5, 13),
    ] {
        items.push(item(next, CLOTHING, name.into(), kind, flags, agent));
        next += 1;
    }
    // Unpacked deliveries: a folder each, moved as a whole.
    for n in 0..24u128 {
        let product = PRODUCTS[n as usize % PRODUCTS.len()];
        let body = ["Reborn", "Legacy", "Maitreya", "eBODY Reborn"][n as usize % 4];
        let id = Uuid::from_u128(8_605_000 + n);
        let name = format!("{} {product} ({body})", STORES[n as usize % STORES.len()]);
        inv.folders.insert(id, folder(id, OBJECTS, &name, -1));
        if let Some(f) = inv.folders.get_mut(&OBJECTS) {
            f.children.push(id);
        }
        for part in ["", " - HUD", " - Texture HUD"] {
            items.push(item(next, id, format!("{product}{part}"), 6, 0, agent));
            next += 1;
        }
    }
    // A fatpack delivered as a folder: its dress, HUD, card and landmark
    // belong together and move as one.
    let fatpack = Uuid::from_u128(8_605_100);
    inv.folders
        .insert(fatpack, folder(fatpack, OBJECTS, "Lumen - Hex Dress - FATPACK", -1));
    if let Some(f) = inv.folders.get_mut(&OBJECTS) {
        f.children.push(fatpack);
    }
    // mixed: one item from 2025, the rest from 2012 (Clean up keeps it whole)
    for (name, kind, year) in [
        ("Lumen - Hex Dress (Reborn Squish) - FATPACK", 6, 17),
        ("Lumen - Hex Dress - FATPACK HUD", 6, 4),
        ("Lumen - INFO CARD", 7, 4),
        ("Lumen - Mainstore", 3, 4),
    ] {
        let mut it = item(next, fatpack, name.into(), kind, 0, agent);
        it.created_at = Y2008 + year * YEAR + 86_400;
        items.push(it);
        next += 1;
    }
    // A delivery entirely from 2010: Clean up takes it whole.
    let retro = Uuid::from_u128(8_605_101);
    inv.folders
        .insert(retro, folder(retro, OBJECTS, "Retro Mall - Skater Outfit - FATPACK", -1));
    if let Some(f) = inv.folders.get_mut(&OBJECTS) {
        f.children.push(retro);
    }
    for (name, kind) in [
        ("Retro Mall - Skater Outfit", 6),
        ("Retro Mall - Skater Outfit HUD", 6),
        ("Retro Mall - read me", 7),
        ("Retro Mall - mainstore", 3),
    ] {
        let mut it = item(next, retro, name.into(), kind, 0, agent);
        it.created_at = Y2008 + 2 * YEAR + 3_600 * next as i64 % 86_400;
        items.push(it);
        next += 1;
    }
    // The user's own folders at the root, with every kind of item, old and
    // new: Clean up opens them and takes their old items one by one.
    for (id, name) in [(OLD_FREEBIES, "Old freebies"), (PLACES, "Places I liked")] {
        inv.folders.insert(id, folder(id, root, name, -1));
        if let Some(f) = inv.folders.get_mut(&root) {
            f.children.push(id);
        }
        for (kind, label) in [
            (7, "Notecard"),
            (3, "Landmark"),
            (0, "Snapshot"),
            (10, "Script"),
            (1, "Sound"),
            (20, "Dance"),
            (21, "Gesture"),
            (6, "Gift"),
        ] {
            for k in 0..3 {
                items.push(item(next, id, format!("{label} {}", k + 1), kind, 0, agent));
                next += 1;
            }
        }
    }
    // An empty box at the root, an empty folder inside it: Clean up's
    // "Include empty folders" takes the box whole.
    for (id, parent, name) in [
        (EMPTY_BOX, root, "Empty box"),
        (Uuid::from_u128(8_605_211), EMPTY_BOX, "Nothing inside"),
    ] {
        inv.folders.insert(id, folder(id, parent, name, -1));
        if let Some(f) = inv.folders.get_mut(&parent) {
            f.children.push(id);
        }
    }
    inv.add_items(items);
    inv.generation += 1;
    inv.sort_all();
}

const OLD_FREEBIES: Uuid = Uuid::from_u128(8_605_200);
const PLACES: Uuid = Uuid::from_u128(8_605_201);
const EMPTY_BOX: Uuid = Uuid::from_u128(8_605_210);

/// `AURORA_DEMO_SORTER_TAB`: rules of every kind, store hints and an
/// excluded folder, so that each tab of the window has something to show.
/// Only the demo's own settings are touched.
pub fn seed_settings(prefs: &mut super::Preferences) {
    use super::{Category, Match, Rule, StoreHint};
    prefs.rules = vec![
        Rule {
            words: "denim jeans".into(),
            mode: Match::Any,
            folder: "Clothes/Denim".into(),
            ..Default::default()
        },
        Rule {
            words: "gift box".into(),
            mode: Match::Phrase,
            target: Some(OLD_FREEBIES),
            ..Default::default()
        },
        Rule {
            words: "freebie demo".into(),
            mode: Match::Any,
            pre_trash: true,
            skip_sorted: true,
            ..Default::default()
        },
    ];
    prefs.store_hints = vec![
        StoreHint {
            store: "Birch Hollow".into(),
            category: Category::Furniture,
        },
        StoreHint {
            store: "Neverlace".into(),
            category: Category::Decor,
        },
    ];
    prefs.excluded = vec![PLACES];
}
