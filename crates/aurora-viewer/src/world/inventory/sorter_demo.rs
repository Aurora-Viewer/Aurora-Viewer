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
];

/// Things with no body, and names the sorter cannot read.
const LOOSE: &[(&str, i32)] = &[
    ("Black leather chair", 6),
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
        created_at: 1_700_000_000 + id as i64,
        ..Default::default()
    }
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
    inv.add_items(items);
    inv.generation += 1;
    inv.sort_all();
}
