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

/// How a rule's words meet a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Match {
    /// Every word, in any order.
    #[default]
    All,
    /// At least one of the words.
    Any,
    /// The words together, in this order.
    Phrase,
}

/// A user rule, tried in order before the built-in body and category rules.
/// It takes an entry by its words, its acquired date (`before`), or both.
/// Its destination is the Pre-Trash folder (`pre_trash`), one of the user's
/// own folders (`target`) when set, otherwise `folder`, a path below the
/// sort folder (`Home/Chairs`) created when missing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    pub enabled: bool,
    pub words: String,
    #[serde(rename = "match")]
    pub mode: Match,
    pub folder: String,
    pub target: Option<Uuid>,
    /// Only entries acquired before this date (`2020`, `2020-06` or
    /// `2020-06-15`); empty for any date.
    pub before: String,
    /// Send what it takes to the Pre-Trash folder, checked before the Trash.
    pub pre_trash: bool,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            enabled: true,
            words: String::new(),
            mode: Match::All,
            folder: String::new(),
            target: None,
            before: String::new(),
            pre_trash: false,
        }
    }
}

/// The start of a day typed as `YYYY`, `YYYY-MM` or `YYYY-MM-DD` (also with
/// `/` or `.`), in seconds since 1970, UTC: what an item's acquired date
/// (`created_at`, the date its Properties show) is compared with.
pub fn date_start(text: &str) -> Option<i64> {
    let parts: Vec<&str> = text.trim().split(['-', '/', '.']).collect();
    if parts.is_empty() || parts.len() > 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let year: i32 = parts[0].parse().ok().filter(|y| (2003..=2100).contains(y))?;
    let month: u32 = parts.get(1).map_or(Some(1), |m| m.parse().ok())?;
    let day: u32 = parts.get(2).map_or(Some(1), |d| d.parse().ok())?;
    let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
    Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp())
}

impl Rule {
    /// The rule can take entries: words, and somewhere to put them.
    /// The rule can take entries: words or a valid date, and somewhere to
    /// put them. A date that does not read leaves the rule incomplete
    /// rather than matching everything.
    pub fn usable(&self) -> bool {
        let dated = !self.before.trim().is_empty();
        self.enabled
            && (dated || !words(&self.words).is_empty())
            && (!dated || date_start(&self.before).is_some())
            && (self.pre_trash || self.target.is_some() || !split_path(&self.folder).is_empty())
    }

    /// `created` is the item's acquired date; a folder has none, so a dated
    /// rule never takes a folder.
    pub fn matches(&self, name_words: &[String], created: Option<i64>) -> bool {
        let wanted = words(&self.words);
        let named = wanted.is_empty()
            || match self.mode {
                Match::All => wanted.iter().all(|w| name_words.contains(w)),
                Match::Any => wanted.iter().any(|w| name_words.contains(w)),
                Match::Phrase => has(name_words, &wanted.join(" ")),
            };
        let dated = match self.before.trim() {
            "" => true,
            text => date_start(text).is_some_and(|limit| created.is_some_and(|c| c > 0 && c < limit)),
        };
        named && dated
    }
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
    /// Squish and Squeeze versions (`sq`, `squish`, `squeeze`) get their
    /// own folder below their category, apart from the regular one.
    pub variants: bool,
    /// A folder per store below the category (`Furniture/North Oak`), read
    /// from the creator's tag ([`store`]).
    pub stores: bool,
    /// Folders Auto-sort never takes anything from, at any depth.
    pub excluded: Vec<Uuid>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            base: String::new(),
            folders: true,
            rules: Vec::new(),
            variants: true,
            stores: false,
            excluded: Vec::new(),
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

    /// `id` is an excluded folder or inside one.
    pub fn excludes(&self, inv: &Inventory, id: Uuid) -> bool {
        !self.excluded.is_empty() && actions::ancestors(inv, id).any(|a| self.excluded.contains(&a))
    }
}

/// The folder at the root of My Inventory that receives what the Pre-Trash
/// rules take: nothing goes to the Trash before the user checks it there.
pub fn pre_trash_name() -> String {
    aurora_i18n::tr!("inventory-sorter-pre-trash").to_owned()
}

/// The Pre-Trash folder, when it exists.
pub fn pre_trash(inv: &Inventory) -> Option<Uuid> {
    child(inv, inv.root, &pre_trash_name())
}

/// What waits in the Pre-Trash folder: its direct contents.
pub fn pre_trash_entries(inv: &Inventory) -> Vec<Uuid> {
    pre_trash(inv)
        .and_then(|id| inv.folders.get(&id))
        .map(|f| f.items.iter().chain(&f.children).copied().collect())
        .unwrap_or_default()
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
    Heads,
    Shapes,
    Animations,
    Huds,
    Furniture,
    Decor,
    Vehicles,
    Pets,
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
        Category::Heads,
        &[
            "head",
            "heads",
            "lelutka",
            "catwa",
            "genus",
            "evox",
            "evo x",
            "akeruka",
            "logo head",
        ],
    ),
    (
        Category::Animations,
        &["pose", "poses", "animation", "animations", "ao", "dance", "posestand"],
    ),
    (Category::Huds, &["hud", "huds"]),
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
        Category::Vehicles,
        &[
            "car",
            "bike",
            "motorbike",
            "motorcycle",
            "boat",
            "yacht",
            "vehicle",
            "truck",
            "scooter",
            "skateboard",
            "helicopter",
            "plane",
        ],
    ),
    (Category::Pets, &["pet", "pets", "puppy", "kitten", "breedable", "companion"]),
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
            Category::Heads => aurora_i18n::tr!("inventory-sorter-heads"),
            Category::Huds => aurora_i18n::tr!("inventory-sorter-huds"),
            Category::Vehicles => aurora_i18n::tr!("inventory-sorter-vehicles"),
            Category::Pets => aurora_i18n::tr!("inventory-sorter-pets"),
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
    /// A Squish or Squeeze version (`Reborn bra black sq`).
    pub squish: bool,
}

const SQUISH_WORDS: &[&str] = &["sq", "squish", "squishy", "squished", "squeeze", "squeez", "squeezed", "squeezy"];

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
    // A texture, a sound, a notecard… is that, whatever its name says
    // ("Bar texture (without chair shadows)" is no chair). Objects,
    // wearables and folders are read from their names first.
    let typed = (!matches!(asset_type, -1 | 5 | 6 | 13))
        .then(|| Category::of_asset(asset_type, flags))
        .flatten();
    let category = typed
        .or_else(|| {
            CATEGORY_WORDS
                .iter()
                .find(|(_, keys)| keys.iter().any(|k| has(&words, k)))
                .map(|(c, _)| *c)
        })
        .or_else(|| Category::of_asset(asset_type, flags));
    Classified {
        body: body(&words),
        category,
        unpacker: UNPACKER_WORDS.iter().any(|k| has(&words, k)),
        squish: SQUISH_WORDS.iter().any(|k| has(&words, k)),
    }
}

/// The first usable user rule that takes `name`, whole words, any case.
pub fn rule_for<'a>(rules: &'a [Rule], name: &str, created: Option<i64>) -> Option<&'a Rule> {
    let words = words(name);
    rules.iter().find(|rule| rule.usable() && rule.matches(&words, created))
}

/// A folder path typed by the user (`Furniture / Chairs`), cleaned the way
/// a new folder's name is.
pub fn split_path(path: &str) -> Vec<String> {
    path.split(['/', '\\'])
        .filter_map(|part| actions::clean_name(part).ok())
        .map(|part| crate::text_validation::generated_name(&part, 63))
        .collect()
}

/// Where an entry goes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Dest {
    /// A path below the sort folder, made when missing.
    Path(Vec<String>),
    /// One of the user's own folders, chosen in a rule.
    Folder(Uuid),
    /// The Pre-Trash folder at the root, made when missing.
    PreTrash,
}

/// The folder path below the base folder, or None when nothing in the name
/// tells where it goes: such entries stay where they are.
#[cfg(test)]
pub fn destination(name: &str, asset_type: i32, flags: u32, rules: &[Rule]) -> Option<Vec<String>> {
    match target(name, asset_type, flags, None, rules, true, false)? {
        Dest::Path(path) => Some(path),
        Dest::Folder(_) | Dest::PreTrash => None,
    }
}

/// Where `name` goes: a user rule first, then the body and the category,
/// and a Squish folder below them for a Squish version (`variants`).
/// `created` is an item's acquired date, for the dated rules.
pub fn target(name: &str, asset_type: i32, flags: u32, created: Option<i64>, rules: &[Rule], variants: bool, stores: bool) -> Option<Dest> {
    if let Some(rule) = rule_for(rules, name, created) {
        return Some(match rule.target {
            _ if rule.pre_trash => Dest::PreTrash,
            Some(id) => Dest::Folder(id),
            None => Dest::Path(split_path(&rule.folder)),
        });
    }
    let found = classify(name, asset_type, flags);
    let mut path = Vec::new();
    path.extend(found.body.map(Body::folder));
    path.extend(found.category.map(|c| c.folder().to_owned()));
    // By store (`stores`): below the category, or in Stores when the name
    // says nothing else.
    if stores && let Some(store) = store(name) {
        if path.is_empty() {
            path.push(aurora_i18n::tr!("inventory-sorter-stores").to_owned());
        }
        path.push(store);
    }
    if variants && found.squish && !path.is_empty() {
        path.push(squish_folder());
    }
    (!path.is_empty()).then_some(Dest::Path(path))
}

/// The store a creator's name starts with: `[North Oak] + Table`,
/// `*Opal&Ash* Dress`, `.VX. Apron`, `Kitoomi - Hex Dress`,
/// `rvn // Boots`. A tag that is a body, a category or an unpacker word
/// (`[REBORN] Skirt`, `Dress - Reborn`) is not a store; neither is a long
/// phrase before a dash, which is a product name.
pub fn store(name: &str) -> Option<String> {
    const ENCLOSED: &[(char, char)] = &[
        ('[', ']'),
        ('{', '}'),
        ('(', ')'),
        ('<', '>'),
        ('«', '»'),
        ('*', '*'),
        ('~', '~'),
        ('.', '.'),
        (':', ':'),
        ('^', '^'),
        ('-', '-'),
        ('+', '+'),
        ('=', '='),
        ('#', '#'),
        ('|', '|'),
    ];
    const SEPARATORS: &[&str] = &[" - ", " // ", " | ", " :: ", " + ", " ~ "];
    let text = name.trim();
    let first = text.chars().next()?;
    let tag = ENCLOSED
        .iter()
        .find(|(open, _)| *open == first)
        .and_then(|(open, close)| {
            let rest = &text[open.len_utf8()..];
            rest.find(*close).map(|end| &rest[..end])
        })
        .or_else(|| {
            SEPARATORS
                .iter()
                .filter_map(|sep| text.find(sep).map(|at| &text[..at]))
                .min_by_key(|tag| tag.len())
        })?;
    let tag = tag.trim_matches(|c: char| !c.is_alphanumeric() && c != '&').trim();
    let tag_words = words(tag);
    let found = classify(tag, -1, 0);
    let reads = (2..=32).contains(&tag.chars().count())
        && (1..=4).contains(&tag_words.len())
        && tag.chars().any(char::is_alphabetic)
        && found.body.is_none()
        && found.category.is_none()
        && !found.unpacker
        && !found.squish;
    reads.then(|| tag.to_owned())
}

fn squish_folder() -> String {
    aurora_i18n::tr!("inventory-sorter-variant-squish").to_owned()
}

/// The names of a folder and its parents, for the preview
/// (`My Inventory / Home / Chairs`).
pub fn folder_path(inv: &Inventory, id: Uuid) -> String {
    let mut names: Vec<String> = actions::ancestors(inv, id)
        .filter_map(|a| inv.folders.get(&a).map(|f| f.info.name.clone()))
        .collect();
    names.reverse();
    names.join(" / ")
}

/// A folder a rule may send entries to: the user's own, not a system
/// folder that refuses moves (`actions::destination`).
pub fn target_allowed(inv: &Inventory, id: Uuid) -> bool {
    inv.folders.get(&id).is_some_and(|f| !f.library) && actions::destination(inv, id) && !untouchable(inv, id)
}

/// One entry to move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    pub id: Uuid,
    pub folder: bool,
    pub name: String,
    /// The folder it is in now: a move whose entry left it is dropped.
    pub from: Uuid,
    pub to: Dest,
    /// The destination as the preview names it.
    pub label: String,
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
            *counts.entry(m.label.clone()).or_default() += 1;
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

/// The Trash and Marketplace Listings, at any depth: Auto-sort never takes
/// anything out of them nor puts anything in (the Trash only through the
/// confirmed Pre-Trash step, the ordinary delete path).
pub fn untouchable(inv: &Inventory, id: Uuid) -> bool {
    actions::in_type(inv, id, 14) || actions::in_type(inv, id, 53)
}

/// Whether the sorter can run on `source`: a folder of the agent's own
/// inventory, outside the Trash and Marketplace Listings.
pub fn source_allowed(inv: &Inventory, source: Uuid) -> bool {
    inv.folders.get(&source).is_some_and(|f| !f.library) && actions::mutable(inv, source) && !untouchable(inv, source)
}

/// The system folders a whole-inventory sort reads, besides the loose
/// entries of My Inventory: those that collect deliveries and loose
/// things. The Trash, Marketplace Listings, the outfits, the Current Outfit,
/// Favorites and the Library are never read.
const WHOLE_TYPES: &[i32] = &[0, 1, 3, 5, 6, 7, 10, 13, 15, 16, 20, 21, 50, 56, 57];

/// The folders a whole-inventory sort takes the direct contents of: My
/// Inventory (its loose items only) and its system folders of
/// [`WHOLE_TYPES`], less the excluded ones. The user's own folders are
/// their organisation: they are neither opened nor moved.
pub fn whole_sources(inv: &Inventory, prefs: &Preferences) -> Vec<Uuid> {
    let mut sources = vec![inv.root];
    if let Some(root) = inv.folders.get(&inv.root) {
        sources.extend(root.children.iter().copied().filter(|id| {
            inv.folders
                .get(id)
                .is_some_and(|f| !f.library && WHOLE_TYPES.contains(&f.info.type_default) && !prefs.excludes(inv, *id))
        }));
    }
    sources
}

/// The plan of a whole-inventory sort (see [`whole_sources`]).
pub fn plan_whole(inv: &Inventory, prefs: &Preferences, protected: &HashSet<Uuid>, worn: &HashSet<Uuid>) -> Plan {
    let mut entries = Vec::new();
    for source in whole_sources(inv, prefs) {
        let folders = prefs.folders && source != inv.root;
        entries.extend(contents(inv, source, prefs, folders));
    }
    plan_entries(inv, &entries, prefs, protected, worn)
}

/// Can `id` leave its folder for the sorted ones? The rules of a drag
/// (`relocatable`), nothing worn and no link (an outfit's link belongs to
/// its folder).
fn movable(inv: &Inventory, id: Uuid, protected: &HashSet<Uuid>, holds_worn: &HashSet<Uuid>) -> bool {
    actions::relocatable(inv, id, protected)
        && !untouchable(inv, id)
        && !holds_worn.contains(&id)
        && inv.items.get(&id).is_none_or(|it| !matches!(it.asset_type, 24 | 25))
}

/// The names the sorter gives its own folders: a folder already named like
/// one is a sorted folder, never moved into another.
fn sorter_folder_names(prefs: &Preferences) -> HashSet<String> {
    let mut names: HashSet<String> = BODIES.iter().map(|b| b.body.to_lowercase()).collect();
    names.insert(Body::Several.folder().to_lowercase());
    names.insert(prefs.base_name().to_lowercase());
    names.insert(pre_trash_name().to_lowercase());
    names.insert(squish_folder().to_lowercase());
    names.insert(aurora_i18n::tr!("inventory-sorter-stores").to_lowercase());
    for (category, _) in CATEGORY_WORDS {
        names.insert(category.folder().to_lowercase());
    }
    for rule in prefs.rules.iter().filter(|r| r.target.is_none()) {
        names.extend(split_path(&rule.folder).into_iter().map(|p| p.to_lowercase()));
    }
    names
}

/// The moves of the direct contents of `source`. A folder goes as a whole
/// (`prefs.folders`), classified by its own name. Entries already in the
/// folder they would go to are left out.
pub fn plan(inv: &Inventory, source: Uuid, prefs: &Preferences, protected: &HashSet<Uuid>, worn: &HashSet<Uuid>) -> Plan {
    let entries = contents(inv, source, prefs, prefs.folders);
    plan_entries(inv, &entries, prefs, protected, worn)
}

/// The direct contents of `source` a sort reads: its items, and its folders
/// when `folders`, less the sorted folders themselves.
fn contents(inv: &Inventory, source: Uuid, prefs: &Preferences, folders: bool) -> Vec<Uuid> {
    let Some(folder) = inv.folders.get(&source) else {
        return Vec::new();
    };
    let base = child(inv, inv.root, &prefs.base_name());
    let reserved = sorter_folder_names(prefs);
    let mut entries: Vec<Uuid> = folder.items.clone();
    if folders {
        // the sorted folders themselves stay where they are
        entries.extend(folder.children.iter().copied().filter(|id| {
            Some(*id) != base
                && inv
                    .folders
                    .get(id)
                    .is_some_and(|f| !reserved.contains(&f.info.name.trim().to_lowercase()))
        }));
    }
    entries
}

/// The moves of chosen entries, wherever they are: the selection a
/// right-click sorts at once. A folder goes as a whole, by its own name.
pub fn plan_entries(inv: &Inventory, entries: &[Uuid], prefs: &Preferences, protected: &HashSet<Uuid>, worn: &HashSet<Uuid>) -> Plan {
    let mut plan = Plan::default();
    let holds_worn = actions::worn_ancestors(inv, worn);
    let base = child(inv, inv.root, &prefs.base_name());
    let mut candidates = Vec::new();
    for id in entries.iter().copied() {
        let (name, asset_type, flags, created, is_folder) = match (inv.folders.get(&id), inv.items.get(&id)) {
            (Some(f), _) => (f.info.name.clone(), -1, 0, None, true),
            (None, Some(it)) => (it.name.clone(), it.asset_type, it.flags, Some(it.created_at), false),
            _ => continue,
        };
        let Some(from) = actions::parent(inv, id) else { continue };
        if movable(inv, id, protected, &holds_worn) && !prefs.excludes(inv, id) {
            candidates.push((id, is_folder, from, name, asset_type, flags, created));
        } else {
            plan.kept += 1;
        }
    }
    // Reading thousands of names against every phrase is the heavy part:
    // it runs on all cores, the inventory itself is only read above.
    use rayon::prelude::*;
    let read: Vec<_> = candidates
        .into_par_iter()
        .map(|(id, folder, from, name, asset_type, flags, created)| {
            let to = target(&name, asset_type, flags, created, &prefs.rules, prefs.variants, prefs.stores);
            let unpacker = classify(&name, asset_type, flags).unpacker;
            (id, folder, from, name, to, unpacker)
        })
        .collect();
    let base_name = prefs.base_name();
    let pre_trash_at = pre_trash(inv);
    let pre_trash_label = format!("{} / {}", folder_path(inv, inv.root), pre_trash_name());
    for (id, folder, from, name, to, unpacker) in read {
        let Some(to) = to else {
            plan.unknown.push(name);
            continue;
        };
        let label = match &to {
            // already sorted: its folder is the destination
            Dest::Path(path) if base.is_some_and(|base| resolve_existing(inv, base, path) == Some(from)) => continue,
            Dest::Folder(target) if *target == from || *target == id => continue,
            Dest::PreTrash if pre_trash_at == Some(from) || pre_trash_at == Some(id) => continue,
            Dest::PreTrash => pre_trash_label.clone(),
            Dest::Folder(target) if !target_allowed(inv, *target) || actions::under(inv, *target, id) => {
                // the rule's folder is gone or cannot receive it
                plan.unknown.push(name);
                continue;
            }
            Dest::Path(path) => std::iter::once(base_name.as_str())
                .chain(path.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(" / "),
            Dest::Folder(target) => folder_path(inv, *target),
        };
        plan.moves.push(Move {
            id,
            folder,
            name,
            from,
            to,
            label,
            unpacker,
        });
    }
    plan.moves.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.name.cmp(&b.name)));
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
/// deleted, moved away from its folder meanwhile) are dropped.
pub fn step(inv: &mut Inventory, base_name: &str, queue: &mut VecDeque<Move>, protected: &HashSet<Uuid>, worn: &HashSet<Uuid>) -> Step {
    let holds_worn = actions::worn_ancestors(inv, worn);
    let mut change = Mutation::default();
    // folders known in this mutation: (parent, lowercase name) → id
    let mut made: HashMap<(Uuid, String), Uuid> = HashMap::new();
    let mut fresh: HashSet<Uuid> = HashSet::new();
    let mut taken = 0;
    let base_path = [base_name.to_owned()];
    'moves: while taken < BATCH {
        let Some(next) = queue.front() else { break };
        let still_here = actions::parent(inv, next.id) == Some(next.from)
            && inv.folders.contains_key(&next.id) == next.folder
            && movable(inv, next.id, protected, &holds_worn);
        if !still_here {
            queue.pop_front();
            continue;
        }
        let path: Vec<String> = match &next.to {
            Dest::Folder(target) => {
                if !target_allowed(inv, *target) {
                    queue.pop_front();
                    continue;
                }
                Vec::new()
            }
            Dest::Path(path) => base_path.iter().chain(path).cloned().collect(),
            Dest::PreTrash => vec![pre_trash_name()],
        };
        let mut parent = match next.to {
            Dest::Folder(target) => target,
            Dest::Path(_) | Dest::PreTrash => inv.root,
        };
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
        change.refresh.extend([parent, m.from]);
        taken += 1;
    }
    if change.operations.is_empty() {
        return Step::Done;
    }
    change.refresh.sort_unstable();
    change.refresh.dedup();
    Step::Change(Box::new(change), taken)
}

#[path = "sorter_demo.rs"]
pub mod demo;

#[cfg(test)]
#[path = "sorter_tests.rs"]
mod tests;
