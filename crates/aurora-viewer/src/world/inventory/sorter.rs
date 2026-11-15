//! Inventory auto-sort: files the loose entries of one folder into
//! `<base>/<body>/<category>` folders, from the names creators give them.
//!
//! The Legacy Viewer has no such tool; it is an Aurora feature. Moving only
//! changes an entry's parent, through the same mutations as a drag in the
//! inventory (`actions::move_selection`): no copy, no rename, no permission
//! change. What a drag may not move (`actions::relocatable`: system, locked,
//! Library, Current Outfit, Received Items) or what is worn stays where it is.
//!
//! The classifier is pure: [`classify`] reads a name and the asset type,
//! [`destination`] turns that into a folder path, [`plan`] lists the moves of a
//! folder and [`step`] builds the next bounded mutation, creating the missing
//! folders in the same request (the network layer maps their new ids).

use super::{FetchState, Inventory, actions};
use aurora_llsd::llsd_map;
use aurora_net::inventory::operations::{Mutation, Operation};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use uuid::Uuid;

/// Moves sent in one mutation: well under the 2048 operations the network
/// layer accepts, and small enough for steady progress on a large inventory.
pub const BATCH: usize = 200;

/// A user rule: names holding every word of `words` go to `folder`, a path
/// below the base folder (`Furniture/Chairs`). Rules are tried in order,
/// before the built-in body and category rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Rule {
    pub words: String,
    pub folder: String,
}

/// Saved with the inventory preferences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// The folder at the root of My Inventory that receives the sorted
    /// folders; empty uses the translated default name.
    pub base: String,
    /// Sort folders too (an unpacked delivery arrives as a folder), as a
    /// whole: their contents are never split.
    pub folders: bool,
    pub rules: Vec<Rule>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            base: String::new(),
            folders: true,
            rules: Vec::new(),
        }
    }
}

impl Preferences {
    pub fn base_name(&self) -> String {
        let name = self.base.trim();
        if name.is_empty() {
            aurora_i18n::tr!("inventory-sorter-default-base").to_owned()
        } else {
            name.to_owned()
        }
    }
}

/// A body (or the several bodies of a fatpack). Its folder carries the
/// creators' own name of the body, the same in every language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Body {
    Named(&'static str),
    Several,
}

impl Body {
    pub fn folder(self) -> String {
        match self {
            Body::Named(name) => name.to_owned(),
            Body::Several => aurora_i18n::tr!("inventory-sorter-several-bodies").to_owned(),
        }
    }
}

/// Each body, the words creators use for it, and the bodies a match rules
/// out (eBODY Reborn is eBODY's Reborn body, not its older Classic one).
/// Weak words only count when no other word names a body: "Lara" or "Juicy"
/// are also ordinary words in item names.
struct BodyWords {
    body: &'static str,
    strong: &'static [&'static str],
    weak: &'static [&'static str],
    hides: &'static [&'static str],
}

const BODIES: &[BodyWords] = &[
    BodyWords {
        body: "Reborn",
        strong: &["reborn", "ebody reborn", "rebornf"],
        weak: &["waifu", "juicy"],
        hides: &["eBODY"],
    },
    BodyWords {
        body: "eBODY",
        strong: &["ebody", "ebody classic", "ebody curvy"],
        weak: &[],
        hides: &[],
    },
    BodyWords {
        body: "Legacy Male",
        strong: &["legacy m", "legacy male", "legacy men", "legacy mens", "legacy boy"],
        weak: &[],
        hides: &["Legacy"],
    },
    BodyWords {
        body: "Legacy",
        strong: &["legacy", "legacy f", "legacy perky", "legacy pinup", "legacyf"],
        weak: &[],
        hides: &[],
    },
    BodyWords {
        body: "Maitreya",
        strong: &["maitreya", "lara x", "lara petite", "maitreya lara", "mait"],
        weak: &["lara"],
        hides: &[],
    },
    BodyWords {
        body: "Belleza Jake",
        strong: &["belleza jake"],
        weak: &["jake"],
        hides: &["Belleza"],
    },
    BodyWords {
        body: "Belleza",
        strong: &["belleza"],
        weak: &["freya", "isis", "venus"],
        hides: &[],
    },
    BodyWords {
        body: "Slink",
        strong: &["slink"],
        weak: &["physique", "hourglass"],
        hides: &[],
    },
    BodyWords {
        body: "Kupra",
        strong: &["kupra", "inithium"],
        weak: &["erika"],
        hides: &[],
    },
    BodyWords {
        body: "Gianni",
        strong: &["gianni", "signature gianni"],
        weak: &[],
        hides: &[],
    },
    BodyWords {
        body: "Geralt",
        strong: &["geralt", "signature geralt"],
        weak: &[],
        hides: &[],
    },
    BodyWords {
        body: "Aesthetic",
        strong: &["aesthetic body"],
        weak: &["aesthetic"],
        hides: &[],
    },
];

/// The category folders, in the order they are tried: the first match wins,
/// so a "Lingerie Set" is lingerie and a "Garter" lingerie before an
/// accessory. Sets come last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    Tattoos,
    Skins,
    Makeup,
    Hair,
    Eyes,
    Nails,
    Shoes,
    Stockings,
    Lingerie,
    Swimwear,
    Bodysuits,
    Dresses,
    Skirts,
    Pants,
    Shorts,
    Jackets,
    Tops,
    Sleepwear,
    Gloves,
    Jewelry,
    Accessories,
    BodyAddons,
    Shapes,
    Animations,
    Furniture,
    Decor,
    Outfits,
    Landmarks,
    Notecards,
    Scripts,
    Sounds,
    Textures,
    Gestures,
}

const CATEGORY_WORDS: &[(Category, &[&str])] = &[
    (Category::Tattoos, &["tattoo", "tattoos", "tatoo", "tatoos", "tattoo layer"]),
    (Category::Skins, &["skin", "skins"]),
    (
        Category::Makeup,
        &[
            "makeup",
            "make up",
            "lipstick",
            "lipsticks",
            "lip gloss",
            "gloss",
            "eyeshadow",
            "eyeshadows",
            "eye shadow",
            "eyeliner",
            "eye liner",
            "liner",
            "blush",
            "brows",
            "eyebrows",
            "freckles",
            "lashes",
            "contour",
            "highlighter",
        ],
    ),
    (
        Category::Hair,
        &["hair", "hairstyle", "hairbase", "hair base", "ponytail", "bangs", "braids", "updo"],
    ),
    (Category::Eyes, &["eyes", "eye", "iris", "irises"]),
    (Category::Nails, &["nails", "nail", "manicure", "pedicure"]),
    (
        Category::Shoes,
        &[
            "shoes",
            "shoe",
            "heels",
            "heel",
            "boots",
            "boot",
            "sneakers",
            "sandals",
            "pumps",
            "stilettos",
            "loafers",
            "slippers",
            "flats",
            "wedges",
            "mules",
        ],
    ),
    (
        Category::Stockings,
        &[
            "stockings",
            "stocking",
            "tights",
            "socks",
            "sock",
            "pantyhose",
            "fishnets",
            "fishnet",
            "thigh highs",
            "knee highs",
            "leg warmers",
        ],
    ),
    (
        Category::Lingerie,
        &[
            "lingerie",
            "bra",
            "bralette",
            "panties",
            "panty",
            "thong",
            "corset",
            "garter",
            "garters",
            "bustier",
            "babydoll",
            "teddy",
            "underwear",
            "briefs",
            "boxers",
            "negligee",
        ],
    ),
    (Category::Swimwear, &["bikini", "swimsuit", "swimwear", "monokini", "bathing suit"]),
    (
        Category::Bodysuits,
        &["bodysuit", "catsuit", "leotard", "jumpsuit", "romper", "playsuit", "overalls"],
    ),
    (Category::Dresses, &["dress", "dresses", "gown", "sundress", "minidress"]),
    (Category::Skirts, &["skirt", "skirts", "miniskirt", "tutu"]),
    (
        Category::Pants,
        &["pants", "jeans", "trousers", "leggings", "joggers", "slacks", "cargos"],
    ),
    (Category::Shorts, &["shorts", "hotpants", "hot pants"]),
    (
        Category::Jackets,
        &[
            "jacket", "jackets", "coat", "coats", "blazer", "parka", "cardigan", "bomber", "trench",
        ],
    ),
    (
        Category::Tops,
        &[
            "top",
            "tops",
            "shirt",
            "shirts",
            "tshirt",
            "tee",
            "blouse",
            "tank",
            "crop top",
            "sweater",
            "hoodie",
            "pullover",
            "vest",
            "cami",
            "turtleneck",
            "jumper",
        ],
    ),
    (
        Category::Sleepwear,
        &[
            "robe",
            "kimono",
            "pajamas",
            "pyjamas",
            "pajama",
            "nightgown",
            "sleepwear",
            "nightie",
        ],
    ),
    (Category::Gloves, &["gloves", "glove", "mittens"]),
    (
        Category::Jewelry,
        &[
            "necklace",
            "necklaces",
            "earrings",
            "earring",
            "ring",
            "rings",
            "bracelet",
            "bracelets",
            "bangles",
            "choker",
            "piercing",
            "piercings",
            "anklet",
            "jewelry",
            "jewellery",
            "pendant",
            "septum",
        ],
    ),
    (
        Category::Accessories,
        &[
            "apron",
            "bag",
            "purse",
            "handbag",
            "backpack",
            "glasses",
            "sunglasses",
            "hat",
            "cap",
            "beanie",
            "belt",
            "scarf",
            "collar",
            "mask",
            "headband",
            "headpiece",
            "crown",
            "tiara",
            "bow",
            "ribbon",
            "harness",
            "leash",
            "wings",
            "tail",
            "ears",
            "horns",
        ],
    ),
    (
        Category::BodyAddons,
        &[
            "mod",
            "mods",
            "addon",
            "add on",
            "addons",
            "thighs",
            "boobs",
            "breasts",
            "butt",
            "rolls",
            "belly",
            "nipples",
            "bump",
            "body reflection",
            "applier",
            "appliers",
        ],
    ),
    (Category::Shapes, &["shape", "shapes"]),
    (
        Category::Animations,
        &["pose", "poses", "animation", "animations", "ao", "dance", "posestand"],
    ),
    (
        Category::Furniture,
        &[
            "chair", "chairs", "sofa", "couch", "table", "tables", "bed", "beds", "desk", "stool", "bench", "shelf", "shelves", "lamp",
            "rug", "cabinet", "armchair", "dresser", "wardrobe", "bathtub", "sink",
        ],
    ),
    (
        Category::Decor,
        &[
            "decor", "plant", "plants", "vase", "frame", "painting", "candle", "candles", "clutter", "curtain", "curtains", "pillow",
            "pillows", "mirror",
        ],
    ),
    (
        Category::Outfits,
        &["outfit", "outfits", "set", "fatpack", "fat pack", "collection", "bundle"],
    ),
];

impl Category {
    pub fn folder(self) -> &'static str {
        match self {
            Category::Tattoos => aurora_i18n::tr!("inventory-sorter-tattoos"),
            Category::Skins => aurora_i18n::tr!("inventory-sorter-skins"),
            Category::Makeup => aurora_i18n::tr!("inventory-sorter-makeup"),
            Category::Hair => aurora_i18n::tr!("inventory-sorter-hair"),
            Category::Eyes => aurora_i18n::tr!("inventory-sorter-eyes"),
            Category::Nails => aurora_i18n::tr!("inventory-sorter-nails"),
            Category::Shoes => aurora_i18n::tr!("inventory-sorter-shoes"),
            Category::Stockings => aurora_i18n::tr!("inventory-sorter-stockings"),
            Category::Lingerie => aurora_i18n::tr!("inventory-sorter-lingerie"),
            Category::Swimwear => aurora_i18n::tr!("inventory-sorter-swimwear"),
            Category::Bodysuits => aurora_i18n::tr!("inventory-sorter-bodysuits"),
            Category::Dresses => aurora_i18n::tr!("inventory-sorter-dresses"),
            Category::Skirts => aurora_i18n::tr!("inventory-sorter-skirts"),
            Category::Pants => aurora_i18n::tr!("inventory-sorter-pants"),
            Category::Shorts => aurora_i18n::tr!("inventory-sorter-shorts"),
            Category::Jackets => aurora_i18n::tr!("inventory-sorter-jackets"),
            Category::Tops => aurora_i18n::tr!("inventory-sorter-tops"),
            Category::Sleepwear => aurora_i18n::tr!("inventory-sorter-sleepwear"),
            Category::Gloves => aurora_i18n::tr!("inventory-sorter-gloves"),
            Category::Jewelry => aurora_i18n::tr!("inventory-sorter-jewelry"),
            Category::Accessories => aurora_i18n::tr!("inventory-sorter-accessories"),
            Category::BodyAddons => aurora_i18n::tr!("inventory-sorter-body-addons"),
            Category::Shapes => aurora_i18n::tr!("inventory-sorter-shapes"),
            Category::Animations => aurora_i18n::tr!("inventory-sorter-animations"),
            Category::Furniture => aurora_i18n::tr!("inventory-sorter-furniture"),
            Category::Decor => aurora_i18n::tr!("inventory-sorter-decor"),
            Category::Outfits => aurora_i18n::tr!("inventory-sorter-outfits"),
            Category::Landmarks => aurora_i18n::tr!("inventory-sorter-landmarks"),
            Category::Notecards => aurora_i18n::tr!("inventory-sorter-notecards"),
            Category::Scripts => aurora_i18n::tr!("inventory-sorter-scripts"),
            Category::Sounds => aurora_i18n::tr!("inventory-sorter-sounds"),
            Category::Textures => aurora_i18n::tr!("inventory-sorter-textures"),
            Category::Gestures => aurora_i18n::tr!("inventory-sorter-gestures"),
        }
    }

    /// The category of an asset type when its name says nothing: a
    /// notecard is a notecard. Clothing and body parts read their wearable
    /// type from the item's flags (LLWearableType::EType in the low byte).
    fn of_asset(asset_type: i32, flags: u32) -> Option<Category> {
        Some(match asset_type {
            0 => Category::Textures,
            1 => Category::Sounds,
            3 => Category::Landmarks,
            7 => Category::Notecards,
            10 => Category::Scripts,
            20 => Category::Animations,
            21 => Category::Gestures,
            5 | 13 => match flags & 0xff {
                0 => Category::Shapes,
                1 => Category::Skins,
                2 => Category::Hair,
                3 => Category::Eyes,
                4 | 10 => Category::Tops,
                5 => Category::Pants,
                6 => Category::Shoes,
                7 => Category::Stockings,
                8 => Category::Jackets,
                9 => Category::Gloves,
                11 => Category::Lingerie,
                12 => Category::Skirts,
                14 => Category::Tattoos,
                _ => return None,
            },
            _ => return None,
        })
    }
}

/// What a name says: the body, the category, and whether the entry is an
/// unpacker (a box to wear or rez that gives the real items).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Classified {
    pub body: Option<Body>,
    pub category: Option<Category>,
    pub unpacker: bool,
}

const UNPACKER_WORDS: &[&str] = &[
    "unpack",
    "unpacker",
    "unpacking",
    "packed",
    "unboxer",
    "unbox",
    "wear to unpack",
    "rez to unpack",
];

/// The lowercase words of a name: creators separate them with anything
/// (`.SB.`, `[REBORN]`, `FemAvg_Dress_Cut_Long_Reborn_F`, `Reborn+Maze`).
pub fn words(name: &str) -> Vec<String> {
    name.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// `phrase` (one or more words) appears in `words`, whole words in order.
/// The tables are lowercase already: nothing is allocated per test, which
/// runs for every phrase of every entry of a large folder.
fn has(words: &[String], phrase: &str) -> bool {
    let len = phrase.split(' ').count();
    words.windows(len).any(|w| w.iter().zip(phrase.split(' ')).all(|(a, b)| a == b))
}

fn body(words: &[String]) -> Option<Body> {
    let strong: Vec<&BodyWords> = BODIES.iter().filter(|b| b.strong.iter().any(|w| has(words, w))).collect();
    let found = if strong.is_empty() {
        BODIES.iter().filter(|b| b.weak.iter().any(|w| has(words, w))).collect()
    } else {
        strong
    };
    let hidden: HashSet<&str> = found.iter().flat_map(|b| b.hides.iter().copied()).collect();
    let mut bodies = found.iter().map(|b| b.body).filter(|b| !hidden.contains(b));
    match (bodies.next(), bodies.next()) {
        (Some(one), None) => Some(Body::Named(one)),
        (Some(_), Some(_)) => Some(Body::Several),
        _ => None,
    }
}

pub fn classify(name: &str, asset_type: i32, flags: u32) -> Classified {
    let words = words(name);
    let category = CATEGORY_WORDS
        .iter()
        .find(|(_, keys)| keys.iter().any(|k| has(&words, k)))
        .map(|(c, _)| *c)
        .or_else(|| Category::of_asset(asset_type, flags));
    Classified {
        body: body(&words),
        category,
        unpacker: UNPACKER_WORDS.iter().any(|k| has(&words, k)),
    }
}

/// The user rule that takes `name`, if any: every word of the rule is a
/// whole word of the name, whatever its case.
pub fn rule_for<'a>(rules: &'a [Rule], name: &str) -> Option<&'a Rule> {
    let words = words(name);
    rules.iter().find(|rule| {
        let wanted = self::words(&rule.words);
        !wanted.is_empty() && !split_path(&rule.folder).is_empty() && wanted.iter().all(|w| words.contains(w))
    })
}

/// A folder path typed by the user (`Furniture / Chairs`), cleaned the way
/// a new folder's name is.
pub fn split_path(path: &str) -> Vec<String> {
    path.split(['/', '\\'])
        .filter_map(|part| actions::clean_name(part).ok())
        .map(|part| crate::text_validation::generated_name(&part, 63))
        .collect()
}

/// The folder path below the base folder, or None when nothing in the name
/// tells where it goes: such entries stay where they are.
pub fn destination(name: &str, asset_type: i32, flags: u32, rules: &[Rule]) -> Option<Vec<String>> {
    if let Some(rule) = rule_for(rules, name) {
        return Some(split_path(&rule.folder));
    }
    let found = classify(name, asset_type, flags);
    let mut path = Vec::new();
    path.extend(found.body.map(Body::folder));
    path.extend(found.category.map(|c| c.folder().to_owned()));
    (!path.is_empty()).then_some(path)
}

/// One entry to move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    pub id: Uuid,
    pub folder: bool,
    pub name: String,
    pub path: Vec<String>,
    pub unpacker: bool,
}

#[derive(Debug, Default, Clone)]
pub struct Plan {
    pub moves: Vec<Move>,
    /// Names that say nothing the sorter knows.
    pub unknown: Vec<String>,
    /// Entries that cannot move: worn, links, locked or system folders.
    pub kept: usize,
}

impl Plan {
    /// The destination folders and how many entries each receives, by path.
    pub fn groups(&self) -> Vec<(String, usize)> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for m in &self.moves {
            *counts.entry(m.path.join(" / ")).or_default() += 1;
        }
        let mut groups: Vec<_> = counts.into_iter().collect();
        groups.sort_by_cached_key(|(path, _)| crate::natural_order::key(path));
        groups
    }

    pub fn unpackers(&self) -> usize {
        self.moves.iter().filter(|m| m.unpacker).count()
    }
}

/// The existing ordinary folder named `name` (any case) in `parent`.
pub fn child(inv: &Inventory, parent: Uuid, name: &str) -> Option<Uuid> {
    let name = name.trim().to_lowercase();
    inv.folders
        .get(&parent)?
        .children
        .iter()
        .filter_map(|id| inv.folders.get(id))
        .filter(|f| !f.library && f.info.parent == parent && f.info.type_default == -1 && f.info.name.trim().to_lowercase() == name)
        .map(|f| f.info.id)
        .min()
}

/// Whether the sorter can run on `source`: a loaded folder of the agent's
/// own inventory that holds something, outside the Trash.
pub fn source_allowed(inv: &Inventory, source: Uuid) -> bool {
    inv.folders.get(&source).is_some_and(|f| !f.library) && actions::mutable(inv, source) && !actions::in_type(inv, source, 14)
}

/// Can `id` leave its folder for the sorted ones? The rules of a drag
/// (`relocatable`), nothing worn and no link (an outfit's link belongs to
/// its folder).
fn movable(inv: &Inventory, id: Uuid, protected: &HashSet<Uuid>, holds_worn: &HashSet<Uuid>) -> bool {
    actions::relocatable(inv, id, protected)
        && !holds_worn.contains(&id)
        && inv.items.get(&id).is_none_or(|it| !matches!(it.asset_type, 24 | 25))
}

/// The names the sorter gives its own folders: a folder already named like
/// one is a sorted folder, never moved into another.
fn sorter_folder_names(prefs: &Preferences) -> HashSet<String> {
    let mut names: HashSet<String> = BODIES.iter().map(|b| b.body.to_lowercase()).collect();
    names.insert(Body::Several.folder().to_lowercase());
    names.insert(prefs.base_name().to_lowercase());
    for (category, _) in CATEGORY_WORDS {
        names.insert(category.folder().to_lowercase());
    }
    for rule in &prefs.rules {
        names.extend(split_path(&rule.folder).into_iter().map(|p| p.to_lowercase()));
    }
    names
}

/// The moves of the direct contents of `source`. A folder goes as a whole
/// (`prefs.folders`), classified by its own name. Entries already in the
/// folder they would go to are left out.
pub fn plan(inv: &Inventory, source: Uuid, prefs: &Preferences, protected: &HashSet<Uuid>, worn: &HashSet<Uuid>) -> Plan {
    let mut plan = Plan::default();
    let Some(folder) = inv.folders.get(&source) else { return plan };
    let holds_worn = actions::worn_ancestors(inv, worn);
    let base = child(inv, inv.root, &prefs.base_name());
    let reserved = sorter_folder_names(prefs);
    let mut entries: Vec<(Uuid, bool)> = folder.items.iter().map(|id| (*id, false)).collect();
    if prefs.folders {
        entries.extend(folder.children.iter().map(|id| (*id, true)));
    }
    let mut candidates = Vec::new();
    for (id, is_folder) in entries {
        let (name, asset_type, flags) = match (is_folder, inv.folders.get(&id), inv.items.get(&id)) {
            (true, Some(f), _) => {
                if Some(id) == base || reserved.contains(&f.info.name.trim().to_lowercase()) {
                    continue;
                }
                (f.info.name.clone(), -1, 0)
            }
            (false, _, Some(it)) => (it.name.clone(), it.asset_type, it.flags),
            _ => continue,
        };
        if movable(inv, id, protected, &holds_worn) {
            candidates.push((id, is_folder, name, asset_type, flags));
        } else {
            plan.kept += 1;
        }
    }
    // Reading thousands of names against every phrase is the heavy part:
    // it runs on all cores, the inventory itself is only read above.
    use rayon::prelude::*;
    let read: Vec<_> = candidates
        .into_par_iter()
        .map(|(id, folder, name, asset_type, flags)| {
            let path = destination(&name, asset_type, flags, &prefs.rules);
            let unpacker = classify(&name, asset_type, flags).unpacker;
            (id, folder, name, path, unpacker)
        })
        .collect();
    for (id, folder, name, path, unpacker) in read {
        let Some(path) = path else {
            plan.unknown.push(name);
            continue;
        };
        // already sorted: its folder is the destination
        if base.is_some_and(|base| resolve_existing(inv, base, &path) == Some(source)) {
            continue;
        }
        plan.moves.push(Move {
            id,
            folder,
            name,
            path,
            unpacker,
        });
    }
    plan.moves.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.name.cmp(&b.name)));
    plan.unknown.sort_by_cached_key(|n| crate::natural_order::key(n));
    plan
}

fn resolve_existing(inv: &Inventory, base: Uuid, path: &[String]) -> Option<Uuid> {
    path.iter().try_fold(base, |parent, part| child(inv, parent, part))
}

/// What the next step of a running sort is.
#[derive(Debug)]
pub enum Step {
    /// The next mutation, and how many moves of the queue it carries.
    Change(Box<Mutation>, usize),
    /// A destination folder is loading: ask again next frame.
    Wait,
    Done,
}

/// The next mutation of a sort: at most [`BATCH`] moves from the front of
/// `queue`, with the folders they need. A folder that exists (any case) is
/// reused; a missing one is created in the same mutation under a new id
/// that later operations name. Entries that can no longer move (worn,
/// deleted, moved away from `source` meanwhile) are dropped.
pub fn step(
    inv: &mut Inventory,
    source: Uuid,
    base_name: &str,
    queue: &mut VecDeque<Move>,
    protected: &HashSet<Uuid>,
    worn: &HashSet<Uuid>,
) -> Step {
    let holds_worn = actions::worn_ancestors(inv, worn);
    let mut change = Mutation::default();
    // folders known in this mutation: (parent, lowercase name) → id
    let mut made: HashMap<(Uuid, String), Uuid> = HashMap::new();
    let mut fresh: HashSet<Uuid> = HashSet::new();
    let mut taken = 0;
    let base_path = [base_name.to_owned()];
    'moves: while taken < BATCH {
        let Some(next) = queue.front() else { break };
        let still_here = actions::parent(inv, next.id) == Some(source)
            && inv.folders.contains_key(&next.id) == next.folder
            && movable(inv, next.id, protected, &holds_worn);
        if !still_here {
            queue.pop_front();
            continue;
        }
        let path: Vec<String> = base_path.iter().chain(&next.path).cloned().collect();
        let mut parent = inv.root;
        // The folders this move would create: kept apart until the whole
        // path resolves, so a wait leaves no half-made path in `change`.
        let mut new_folders = Vec::new();
        for part in path {
            let key = (parent, part.to_lowercase());
            if let Some(id) = made.get(&key) {
                parent = *id;
                continue;
            }
            if !fresh.contains(&parent) {
                match inv.folders.get(&parent).map(|f| f.state) {
                    Some(FetchState::Fetched) => {}
                    Some(FetchState::Failed) | None => {
                        // the destination cannot be read: this entry stays
                        queue.pop_front();
                        continue 'moves;
                    }
                    Some(FetchState::Unknown | FetchState::Fetching) => {
                        inv.request(parent);
                        if change.operations.is_empty() {
                            return Step::Wait;
                        }
                        break 'moves;
                    }
                }
                if let Some(id) = child(inv, parent, &part) {
                    made.insert(key, id);
                    parent = id;
                    continue;
                }
            }
            let id = Uuid::new_v4();
            new_folders.push(Operation::Folder {
                id,
                parent,
                name: part,
                type_default: -1,
            });
            change.refresh.push(id);
            made.insert(key, id);
            fresh.insert(id);
            parent = id;
        }
        change.operations.append(&mut new_folders);
        let Some(m) = queue.pop_front() else { break };
        if parent == m.id || actions::under(inv, parent, m.id) {
            continue;
        }
        change.operations.push(Operation::Patch {
            id: m.id,
            folder: m.folder,
            body: llsd_map! { "parent_id" => parent },
        });
        change.refresh.push(parent);
        taken += 1;
    }
    if change.operations.is_empty() {
        return Step::Done;
    }
    change.refresh.push(source);
    change.refresh.sort_unstable();
    change.refresh.dedup();
    Step::Change(Box::new(change), taken)
}

#[path = "sorter_demo.rs"]
pub mod demo;

#[cfg(test)]
#[path = "sorter_tests.rs"]
mod tests;
