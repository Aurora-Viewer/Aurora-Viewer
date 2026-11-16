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
use aurora_net::inventory::InvItem;
use aurora_net::inventory::operations::{Mutation, Operation};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use uuid::Uuid;
use vocab::{BodyWords, VOCABULARY};

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
/// It takes an entry by its words (dates belong to Clean up: `before` is
/// only read so older settings still load, and never matched).
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
    /// The first version's date filter, kept so older settings load; never
    /// matched (a rule takes entries by its words only).
    pub before: String,
    /// Send what it takes to the Pre-Trash folder, checked before the Trash.
    pub pre_trash: bool,
    /// A Pre-Trash rule reads everything in the folder sorted, at any depth,
    /// and the sort folder's tree; this one leaves the sort folder's tree out
    /// ("Everywhere except Sorted").
    pub skip_sorted: bool,
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
            skip_sorted: false,
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
    /// The rule can take entries: words, and somewhere to put them. The
    /// date of the first version (`before`) is no longer read: dates belong
    /// to Clean up ([`Cleanup`]), and an old dated rule without words stays
    /// incomplete rather than taking everything.
    pub fn usable(&self) -> bool {
        self.enabled
            && !joined_words(&self.words).is_empty()
            && (self.pre_trash || self.target.is_some() || !split_path(&self.folder).is_empty())
    }

    /// The rule's words meet the name's.
    pub fn matches(&self, name_words: &[String], _created: Option<i64>) -> bool {
        self.named(name_words)
    }

    /// The words alone.
    pub fn named(&self, name_words: &[String]) -> bool {
        // a rule's own words are read joined (`women's` is `womens`): its
        // parts (`s`) would meet every name with an apostrophe
        let wanted = joined_words(&self.words);
        !wanted.is_empty()
            && match self.mode {
                Match::All => wanted.iter().all(|w| name_words.contains(w)),
                Match::Any => wanted.iter().any(|w| name_words.contains(w)),
                // a phrase reads both ways, like the name: `red women's`
                // meets `red women's dress` as it always did
                Match::Phrase => has(name_words, &wanted.join(" ")) || has(name_words, &words(&self.words).join(" ")),
            }
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
    /// A folder per store below the category (`Furniture/North Oak`), read
    /// from the creator's tag ([`store`]).
    pub stores: bool,
    /// Folders Auto-sort never takes anything from, at any depth.
    pub excluded: Vec<Uuid>,
    /// The Clean up tab's choices.
    pub cleanup: Cleanup,
    /// The item kinds a sort moves; the others stay where they are (a
    /// right-click on chosen entries sorts them whatever their kind).
    pub kinds: Vec<Kind>,
    /// Store → category, for the stores whose names say nothing.
    pub store_hints: Vec<StoreHint>,
    /// Default folders the user allowed Auto-sort into, with a warning:
    /// only [`unlockable`] ones count. Kept per account with the other
    /// options, never shared nor imported.
    pub unlocked: Vec<Uuid>,
    /// Lost and Found's own switch, off by default.
    pub include_lost_found: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            base: String::new(),
            folders: true,
            rules: Vec::new(),
            stores: false,
            excluded: Vec::new(),
            cleanup: Cleanup::default(),
            kinds: Kind::ALL.to_vec(),
            store_hints: Vec::new(),
            unlocked: Vec::new(),
            include_lost_found: false,
        }
    }
}

impl Preferences {
    /// The folders the hands-off rule opens for the user: the unlocked
    /// ones that may be, and Lost and Found when its switch is on.
    pub fn unlocked_set(&self, inv: &Inventory) -> HashSet<Uuid> {
        let mut set: HashSet<Uuid> = self
            .unlocked
            .iter()
            .copied()
            .filter(|id| inv.folders.get(id).is_some_and(|f| unlockable(f.info.type_default)))
            .collect();
        if self.include_lost_found
            && let Some(lost) = actions::system(inv, LOST_AND_FOUND)
        {
            set.insert(lost);
        }
        set
    }

    /// The sort folder's name, as the inventory keeps it. A name that would
    /// reach somewhere it must not (a `#` viewer folder, the Pre-Trash
    /// folder, a default folder's name) is refused for the default one:
    /// typed or imported, it never puts the sort in a hands-off place.
    pub fn base_name(&self) -> String {
        let default = inventory_name(aurora_i18n::tr!("inventory-sorter-default-base"), "Sorted");
        let name = inventory_name(self.base.trim(), &default);
        if base_refused(&name) { default } else { name }
    }

    /// `id` is an excluded folder or inside one.
    pub fn excludes(&self, inv: &Inventory, id: Uuid) -> bool {
        !self.excluded.is_empty() && actions::ancestors(inv, id).any(|a| self.excluded.contains(&a))
    }
}

/// The folder at the root of My Inventory that receives what the Pre-Trash
/// rules take: nothing goes to the Trash before the user checks it there.
pub fn pre_trash_name() -> String {
    inventory_name(aurora_i18n::tr!("inventory-sorter-pre-trash"), "Pre-Trash")
}

/// The names a sort folder may not take: a viewer folder, Pre-Trash (in any
/// language Aurora speaks), My Inventory's default folders.
pub fn base_refused(name: &str) -> bool {
    const DEFAULTS: &[&str] = &[
        "pre-trash",
        "pre-corbeille",
        "trash",
        "objects",
        "clothing",
        "body parts",
        "landmarks",
        "notecards",
        "textures",
        "scripts",
        "sounds",
        "animations",
        "gestures",
        "photo album",
        "calling cards",
        "favorites",
        "materials",
        "settings",
        "lost and found",
        "my outfits",
        "current outfit",
        "received items",
        "marketplace listings",
        "library",
        "my inventory",
    ];
    let lower = name.trim().to_lowercase();
    lower.is_empty()
        || VIEWER_FOLDERS.iter().any(|name| name.eq_ignore_ascii_case(&lower))
        || pre_trash_names().iter().any(|pre| pre.eq_ignore_ascii_case(&lower))
        || DEFAULTS.contains(&lower.as_str())
}

/// A folder name Auto-sort gives, in every language the viewer speaks, as
/// the inventory keeps it: after a change of language, the folders made in
/// the old one are still recognised.
fn every_language(key: &'static str, fallback: &str) -> Vec<String> {
    let mut names: Vec<String> = aurora_i18n::available_languages()
        .map(|(code, _)| inventory_name(aurora_i18n::text_in(code, key), fallback))
        .filter(|name| !name.is_empty())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Pre-Trash's name in every language the viewer speaks (read once).
fn pre_trash_names() -> &'static [String] {
    static NAMES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| every_language("inventory-sorter-pre-trash", "Pre-Trash"))
}

/// The default sort folder's name in every language (read once).
fn default_base_names() -> &'static [String] {
    static NAMES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| every_language("inventory-sorter-default-base", "Sorted"))
}

/// The Pre-Trash folder, when it exists: at the root, by its name in the
/// language in use, else in another one the viewer speaks.
pub fn pre_trash(inv: &Inventory) -> Option<Uuid> {
    child(inv, inv.root, &pre_trash_name()).or_else(|| pre_trash_names().iter().find_map(|name| child(inv, inv.root, name)))
}

/// Two store names are one store: as folder names when both have one,
/// else as written (two Cyrillic names never match as two empty ones).
fn same_store(a: &str, b: &str) -> bool {
    let (x, y) = (inventory_name(a.trim(), ""), inventory_name(b.trim(), ""));
    if x.is_empty() || y.is_empty() {
        a.trim().to_lowercase() == b.trim().to_lowercase()
    } else {
        x.eq_ignore_ascii_case(&y)
    }
}

/// Every Pre-Trash folder: the one in use, and those made in another
/// language at the root (never read as a source, never taken whole).
/// Worked out once per plan, never per entry.
pub fn pre_trash_folders(inv: &Inventory) -> HashSet<Uuid> {
    let in_use = pre_trash_name();
    let mut found: HashSet<Uuid> = inv
        .folders
        .get(&inv.root)
        .map(|root| {
            root.children
                .iter()
                .copied()
                .filter(|id| {
                    inv.folders.get(id).is_some_and(|f| {
                        let name = f.info.name.trim();
                        f.info.type_default == -1
                            && (name.eq_ignore_ascii_case(&in_use) || pre_trash_names().iter().any(|n| name.eq_ignore_ascii_case(n)))
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    found.extend(pre_trash(inv));
    found
}

/// The sort folder, when it exists: by its name; for the default name, in
/// the language in use, else in another one the viewer speaks (a folder
/// named by the user is only ever that name).
pub fn base_folder(inv: &Inventory, prefs: &Preferences) -> Option<Uuid> {
    child(inv, inv.root, &prefs.base_name()).or_else(|| {
        prefs
            .base
            .trim()
            .is_empty()
            .then(|| default_base_names().iter().find_map(|name| child(inv, inv.root, name)))
            .flatten()
    })
}

/// The sort folder's name as the inventory has it (an existing one made in
/// another language included), else the name it will be given.
pub fn base_name_in(inv: &Inventory, prefs: &Preferences) -> String {
    base_folder(inv, prefs)
        .and_then(|id| inv.folders.get(&id))
        .map_or_else(|| prefs.base_name(), |f| f.info.name.clone())
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
            Body::Several => inventory_name(aurora_i18n::tr!("inventory-sorter-several-bodies"), "Several bodies"),
        }
    }
}

/// The category folders. The vocabulary's order (`vocabulary.json`) is the
/// order they are tried: the first match wins (a "Lingerie Set" is
/// lingerie), unless another category has a longer phrase built on the word
/// that matched (a "Hair Bow" is an accessory). Sets come last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
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
    /// Made for Reborn's Waifus mod (`Reborn Waifus - Bra`).
    pub waifus: bool,
}

/// The lowercase words of a name: creators separate them with anything
/// (`.QW.`, `[REBORN]`, `FemAvg_Dress_Cut_Long_Reborn_F`, `Reborn+Luma`).
pub fn words(name: &str) -> Vec<String> {
    // a word with an apostrophe is read both ways: its parts (`kat`, `s`,
    // as rules written before matched it) then the joined word (`kats`,
    // `womens`), so `women's boots` still reads `womens boots`
    let mut out = Vec::new();
    for chunk in name.split(|c: char| !c.is_alphanumeric() && !matches!(c, '\'' | '’')) {
        let parts: Vec<String> = chunk.split(['\'', '’']).filter(|w| !w.is_empty()).map(str::to_lowercase).collect();
        if parts.len() > 1 {
            out.extend(parts.iter().cloned());
            out.push(parts.concat());
        } else {
            out.extend(parts);
        }
    }
    out
}

/// The words of a rule or a tag: an apostrophe joins its word (`women's`
/// is `womens`), never split into parts that would meet any name.
pub fn joined_words(name: &str) -> Vec<String> {
    name.split(|c: char| !c.is_alphanumeric() && !matches!(c, '\'' | '’'))
        .map(|chunk| {
            chunk
                .chars()
                .filter(|c| !matches!(c, '\'' | '’'))
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Two folder names are the same, case and outer spaces aside; `lower` is
/// lowercase already.
fn same_name(name: &str, lower: &str) -> bool {
    let name = name.trim();
    if name.is_ascii() && lower.is_ascii() {
        name.eq_ignore_ascii_case(lower)
    } else {
        name.to_lowercase() == lower
    }
}

/// `phrase` (one or more words) appears in `words`, whole words in order.
/// The tables are lowercase already: nothing is allocated per test, which
/// runs for every phrase of every entry of a large folder.
fn has(words: &[String], phrase: &str) -> bool {
    let len = phrase.split(' ').count();
    words.windows(len).any(|w| w.iter().zip(phrase.split(' ')).all(|(a, b)| a == b))
}

fn body(words: &[String]) -> Option<Body> {
    let bodies = &VOCABULARY.bodies;
    let strong: Vec<&'static BodyWords> = bodies.iter().filter(|b| b.strong.iter().any(|w| has(words, w))).collect();
    let found = if strong.is_empty() {
        bodies.iter().filter(|b| b.weak.iter().any(|w| has(words, w))).collect()
    } else {
        strong
    };
    let hidden: HashSet<&str> = found.iter().flat_map(|b| b.hides.iter().map(String::as_str)).collect();
    let mut bodies = found.iter().map(|b| b.body.as_str()).filter(|b| !hidden.contains(b));
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
            // the first category in the vocabulary's order, unless another
            // has a longer phrase built on the word it matched: `hair bow`
            // is an accessory, not hair; `dress shirt` a top; `cat eye`
            // sunglasses, not eyes. Another category's own words never
            // override it (`tattoo … evo x` stays a tattoo).
            let categories = &VOCABULARY.categories;
            let (first, keys) = categories.iter().find(|(_, keys)| keys.iter().any(|k| has(&words, k)))?;
            let matched: Vec<&String> = keys.iter().filter(|k| has(&words, k)).collect();
            let longer = categories
                .iter()
                .filter(|(c, _)| c != first)
                .flat_map(|(c, keys)| keys.iter().map(move |k| (*c, k)))
                // (a phrase found in the name first: nothing is allocated
                // for the others)
                .filter(|(_, k)| k.contains(' ') && has(&words, k))
                .filter(|(_, k)| {
                    let phrase: Vec<String> = k.split(' ').map(str::to_owned).collect();
                    // the word it extends, and nowhere else in the name
                    // (`Kiara Hair w/ Hair Flower` is still hair)
                    matched.iter().any(|m| {
                        m.split(' ').count() < phrase.len()
                            && has(&phrase, m)
                            && words.iter().filter(|w| *w == *m).count() <= phrase.iter().filter(|w| *w == *m).count()
                    })
                })
                .max_by_key(|(_, k)| k.split(' ').count());
            Some(longer.map_or(*first, |(c, _)| c))
        })
        .or_else(|| Category::of_asset(asset_type, flags));
    let body = body(&words);
    // A body skin says which body and how it looks, not what it is:
    // `The "Harbor Body" for eBody Reborn (Porcelain)`.
    let category = category.or_else(|| {
        let skin = has(&words, "body for") || VOCABULARY.skin_tones.iter().any(|k| has(&words, k));
        (body.is_some() && skin).then_some(Category::Skins)
    });
    Classified {
        body,
        category,
        unpacker: VOCABULARY.unpackers.iter().any(|k| has(&words, k)),
        squish: VOCABULARY.variants.squish.iter().any(|k| has(&words, k)),
        waifus: VOCABULARY.variants.waifus.iter().any(|k| has(&words, k)),
    }
}

/// A user's hint for a store whose names say nothing of what it sells:
/// its items without a category of their own go to `category`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StoreHint {
    pub store: String,
    pub category: Category,
    /// A creator hint: the items this resident made belong to `store`
    /// (their folder per store) and to `category` when their name gives
    /// none; it beats the store read from the name. A creator's id is
    /// public (Properties shows it), never an asset id.
    pub creator: Option<Uuid>,
    /// The creator's name as it was shown when the hint was made.
    pub creator_name: String,
}

impl Default for StoreHint {
    fn default() -> Self {
        Self {
            store: String::new(),
            category: Category::Furniture,
            creator: None,
            creator_name: String::new(),
        }
    }
}

impl Category {
    /// Every category, in folder order, for the store hints' list.
    pub const ALL: [Category; 37] = [
        Category::Tattoos,
        Category::Skins,
        Category::Makeup,
        Category::Hair,
        Category::Eyes,
        Category::Nails,
        Category::Shoes,
        Category::Stockings,
        Category::Lingerie,
        Category::Swimwear,
        Category::Bodysuits,
        Category::Dresses,
        Category::Skirts,
        Category::Pants,
        Category::Shorts,
        Category::Jackets,
        Category::Tops,
        Category::Sleepwear,
        Category::Gloves,
        Category::Jewelry,
        Category::Accessories,
        Category::BodyAddons,
        Category::Heads,
        Category::Shapes,
        Category::Animations,
        Category::Huds,
        Category::Furniture,
        Category::Decor,
        Category::Vehicles,
        Category::Pets,
        Category::Outfits,
        Category::Landmarks,
        Category::Notecards,
        Category::Scripts,
        Category::Sounds,
        Category::Textures,
        Category::Gestures,
    ];
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
        // accents first (`Robes d'été` → `Robes d'ete`), never `?`
        .map(|part| {
            // accents become letters, letter by letter: one the table lacks
            // (`Ł` of `Ładne rzeczy`) is left out, never a `?`, and the
            // rest of the part stays (`adne rzeczy`)
            let latin: String = part
                .chars()
                .map(|c| match c {
                    c if c.is_ascii() => c.to_string(),
                    c if c.is_whitespace() => " ".to_owned(),
                    '’' | '‘' => "'".to_owned(),
                    '–' | '—' => "-".to_owned(),
                    c => inventory_name(c.encode_utf8(&mut [0; 4]), ""),
                })
                .collect();
            let latin = latin.split_whitespace().collect::<Vec<_>>().join(" ");
            if !latin.chars().any(char::is_alphanumeric) {
                return String::new();
            }
            inventory_name(&crate::text_validation::generated_name(&latin, 63), "")
        })
        .filter(|part| !part.is_empty())
        // a rule's path is a few folders, never a hundred nested ones
        .take(MAX_PATH_DEPTH)
        .collect()
}

/// Folders asked for at once while a folder to move waits for its tree.
const LOADS_AT_ONCE: usize = 16;

/// The most folders a rule's path makes below the sort folder.
const MAX_PATH_DEPTH: usize = 8;

/// Where an entry goes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
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
    match target(name, asset_type, flags, None, rules, false)? {
        Dest::Path(path) => Some(path),
        Dest::Folder(_) | Dest::PreTrash => None,
    }
}

/// Where `name` goes: a user rule first, then the body and the category,
/// and always a Squish folder below them for a Squish or Squeeze version.
/// `created` is an item's acquired date, for the dated rules.
pub fn target(name: &str, asset_type: i32, flags: u32, created: Option<i64>, rules: &[Rule], stores: bool) -> Option<Dest> {
    target_with(name, asset_type, flags, created, rules, stores, &[])
}

/// [`target`] with the user's store hints: a store's items whose names give
/// no category take the hint's.
pub fn target_with(
    name: &str,
    asset_type: i32,
    flags: u32,
    created: Option<i64>,
    rules: &[Rule],
    stores: bool,
    hints: &[StoreHint],
) -> Option<Dest> {
    target_by(name, asset_type, flags, created, rules, stores, hints, None)
}

/// [`target_with`] knowing who made the entry: a creator hint gives its
/// store and, when the name gives no category, its category.
#[allow(clippy::too_many_arguments)]
pub fn target_by(
    name: &str,
    asset_type: i32,
    flags: u32,
    created: Option<i64>,
    rules: &[Rule],
    stores: bool,
    hints: &[StoreHint],
    creator: Option<Uuid>,
) -> Option<Dest> {
    if let Some(rule) = rule_for(rules, name, created) {
        return Some(match rule.target {
            _ if rule.pre_trash => Dest::PreTrash,
            Some(id) => Dest::Folder(id),
            None => Dest::Path(split_path(&rule.folder)),
        });
    }
    let mut found = classify(name, asset_type, flags);
    let by_creator = creator.and_then(|c| hints.iter().find(|h| h.creator == Some(c)));
    let mut shop = (stores || !hints.is_empty()).then(|| store(name)).flatten();
    if let Some(hint) = by_creator {
        if !hint.store.trim().is_empty() {
            shop = Some(hint.store.trim().to_owned());
        }
        if found.category.is_none() {
            found.category = Some(hint.category);
        }
    }
    if found.category.is_none()
        && let Some(shop) = &shop
        && let Some(hint) = hints.iter().find(|h| h.creator.is_none() && same_store(&h.store, shop))
    {
        found.category = Some(hint.category);
    }
    let mut path = Vec::new();
    // a body says who wears it: a sofa, a car, a notecard has none
    // (`Hourglass Vase` is no Slink item)
    let worn = !matches!(
        found.category,
        Some(
            Category::Furniture
                | Category::Decor
                | Category::Vehicles
                | Category::Pets
                | Category::Landmarks
                | Category::Notecards
                | Category::Scripts
                | Category::Sounds
                | Category::Textures
        )
    );
    path.extend(found.body.filter(|_| worn).map(Body::folder));
    path.extend(found.category.map(category_folder));
    // By store (`stores`): below the category, or in Stores when the name
    // says nothing else.
    // a creator hint names a store on purpose: its folder is made even with
    // the store option off
    let hinted = by_creator.is_some_and(|h| !h.store.trim().is_empty());
    if (stores || hinted)
        && let Some(shop) = shop
    {
        // a store name with no letter the inventory keeps (`[桜屋]`) gets
        // no folder of its own, and no bare Stores folder either
        let shop = inventory_name(&shop, "");
        if !shop.is_empty() {
            if path.is_empty() {
                path.push(stores_folder());
            }
            path.push(shop);
        }
    }
    // Waifus and Squish versions of clothes get their folder below the
    // category; the Waifus mod itself is a body add-on and gets none.
    let mod_itself = found.category == Some(Category::BodyAddons);
    if !path.is_empty() && !mod_itself && (found.squish || found.waifus) {
        path.push(variant_folder(found.waifus, found.squish));
    }
    (!path.is_empty()).then_some(Dest::Path(path))
}

/// `Waifus`, `Squish` or `Waifus Squish`: creators' words, like the bodies.
fn variant_folder(waifus: bool, squish: bool) -> String {
    match (waifus, squish) {
        (true, true) => format!("{WAIFUS} {}", squish_folder()),
        (true, false) => WAIFUS.to_owned(),
        _ => squish_folder(),
    }
}

const WAIFUS: &str = "Waifus";

/// The store a creator's name starts with: `[North Oak] + Table`,
/// `*Opal&Ash* Dress`, `.QZ. Apron`, `Velmora - Hex Dress`,
/// `zph // Boots`. A tag that is a body, a category or an unpacker word
/// (`[REBORN] Skirt`, `Dress - Reborn`) is not a store; neither is a long
/// phrase before a dash, which is a product name.
pub fn store<'a>(name: &'a str) -> Option<String> {
    let text = name.trim();
    let first = text.chars().next()?;
    // `oh-my: Thigh Socks`, `MARROW: The Body`: a store and a colon
    let tags = &VOCABULARY.store_tags;
    // `**Store** Dress`, `..::Store::.. Top`: the opening run of marks is
    // skipped before the closing one is looked for
    let enclosed = tags.enclosed.iter().find(|(open, _)| *open == first).and_then(|(open, close)| {
        let rest = text[open.len_utf8()..].trim_start_matches(|c: char| c == *open || (!c.is_alphanumeric() && c != '&'));
        // the tag, and the name after it
        rest.find(*close)
            .map(|end| (&rest[..end], rest[end + close.len_utf8()..].trim_start()))
    });
    let separated = |text: &'a str| {
        tags.separators
            .iter()
            .filter_map(|sep| text.find(sep).map(|at| &text[..at]))
            .min_by_key(|tag| tag.len())
    };
    // each way of writing a store in turn: `(Mesh) Velmora - Dress` is no
    // store in its brackets, then one before its dash after them
    [
        enclosed.map(|(t, _)| (t, true)),
        enclosed.and_then(|(_, after)| separated(after)).map(|t| (t, false)),
        separated(text).map(|t| (t, false)),
        underscore_store(text).map(|t| (t, false)),
    ]
    .into_iter()
    .flatten()
    .find_map(|(tag, enclosed)| store_tag(tag, enclosed))
}

/// A bracketed tag says more than a status and what the item is: `[Moon
/// Hair]` keeps `moon`, `[Mesh Dress]` or `[Rigged Hair]` nothing.
fn names_more_than_kind(tag_words: &[String]) -> bool {
    let is_kind = |w: &String| VOCABULARY.categories.iter().any(|(_, keys)| keys.iter().any(|k| k == w));
    tag_words.len() >= 2 && tag_words.iter().any(|w| !VOCABULARY.store_stopwords.contains(w) && !is_kind(w))
}

/// `tag` read as a store's name, when it is one.
fn store_tag(tag: &str, enclosed: bool) -> Option<String> {
    let tag = tag.trim_matches(|c: char| !c.is_alphanumeric() && c != '&').trim();
    let tag_words = joined_words(tag);
    let found = classify(tag, -1, 0);
    // a body mod maker's tag (`LUMA.mods`) names its trade, not a product
    let maker = tag_words.last().is_some_and(|w| w == "mods" || w == "mod");
    let reads = (2..=32).contains(&tag.chars().count())
        && (1..=4).contains(&tag_words.len())
        && tag.chars().any(char::is_alphabetic)
        && found.body.is_none()
        // `[Moon Hair]`, `[Mila Poses]`: a bracketed name of a few words
        // is the store's, even with a category word in it
        && (found.category.is_none() || maker || enclosed && names_more_than_kind(&tag_words))
        && !found.unpacker
        && !found.squish
        && !found.waifus
        // `(Mesh)`, `[DEMO]`, `*NEW*`: a status, not a store
        && !tag_words.iter().all(|w| VOCABULARY.store_stopwords.contains(w));
    reads.then(|| tag.to_owned())
}

/// `OnlyShop_Sign_F`: a name with no space, its parts joined by `_`, starts
/// with its store when that first part is written like a brand (inner
/// capitals, `OnlyShop`, or all capitals, `XYZ`), not like a word (`Akira`).
fn underscore_store(text: &str) -> Option<&str> {
    if text.contains(' ') {
        return None;
    }
    let (first, rest) = text.split_once('_')?;
    let letters: Vec<char> = first.chars().filter(|c| c.is_alphabetic()).collect();
    let inner_capital = letters.iter().skip(1).any(|c| c.is_uppercase()) && letters.iter().any(|c| c.is_lowercase());
    let all_capitals = letters.len() >= 2 && letters.iter().all(|c| c.is_uppercase());
    (!rest.is_empty() && (inner_capital || all_capitals)).then_some(first)
}

fn stores_folder() -> String {
    inventory_name(aurora_i18n::tr!("inventory-sorter-stores"), "Stores")
}

fn squish_folder() -> String {
    inventory_name(aurora_i18n::tr!("inventory-sorter-variant-squish"), "Squish")
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
    folder_open(inv, id) && actions::destination(inv, id) && !in_pre_trash(inv, id) && !in_default_folder(inv, id)
}

/// `id` is a default folder other than Objects (Clothing, Lost and Found…)
/// or inside one. Unlocking such a folder lets Auto-sort take entries out
/// of it, never file entries into it: a rule never sends anything there.
fn in_default_folder(inv: &Inventory, id: Uuid) -> bool {
    std::iter::once(id)
        .chain(actions::ancestors(inv, id))
        .filter(|a| *a != inv.root)
        .any(|a| {
            inv.folders
                .get(&a)
                .is_some_and(|f| !matches!(f.info.type_default, -1 | OPEN_DEFAULT))
        })
}

/// `id` is the Pre-Trash folder or inside it: what is there is on its way
/// to the Trash, never a place to file things.
fn in_pre_trash(inv: &Inventory, id: Uuid) -> bool {
    pre_trash_folders(inv).iter().any(|p| id == *p || actions::under(inv, id, *p))
}

/// Where Undo or a snapshot may put an entry back: any rule destination,
/// and the Pre-Trash folder itself (undoing "Move to Trash" puts its
/// entries back there, where the user left them).
pub fn restore_target_allowed(inv: &Inventory, id: Uuid) -> bool {
    // (inside an unlocked default folder too: an entry goes back where it
    // was taken from)
    folder_open(inv, id) && actions::destination(inv, id) && !in_pre_trash(inv, id)
        || pre_trash(inv) == Some(id) && folder_open(inv, id) && actions::destination(inv, id)
        // the ordinary default folder an entry came from (Animations,
        // Clothing…), locked again since the run: putting it back is a drag
        // back where it was. Never the critical ones (the Trash, outfits,
        // Marketplace…), which are not unlockable.
        || inv.folders.get(&id).is_some_and(|f| {
            !f.library
                && f.info.parent == inv.root
                && (unlockable(f.info.type_default) || f.info.type_default == LOST_AND_FOUND)
                && actions::destination(inv, id)
        })
}

/// A folder below `id` that Auto-sort may not carry along when `id` moves
/// or goes to the Trash whole: a default or typed folder (an outfit, a
/// stock folder…), a viewer folder, or one the user excluded.
fn holds_closed(inv: &Inventory, id: Uuid, excluded: &[Uuid]) -> bool {
    let Some(top) = inv.folders.get(&id) else { return false };
    // not loaded: a worn item or a typed folder could be inside, unseen
    if top.state != FetchState::Fetched {
        return true;
    }
    let mut stack: Vec<Uuid> = top.children.clone();
    let mut seen = HashSet::new();
    while let Some(child) = stack.pop() {
        if !seen.insert(child) {
            continue;
        }
        let Some(f) = inv.folders.get(&child) else { continue };
        if f.info.type_default != -1 || f.library || excluded.contains(&child) || f.state != FetchState::Fetched {
            return true;
        }
        stack.extend(f.children.iter().copied());
    }
    false
}

/// The folders below `id` (itself included) not loaded yet: a folder moves
/// whole once they are, so they are asked for. One the server failed to
/// give is not listed: it keeps its parent where it is (`holds_closed`)
/// and is never asked for again in a loop.
pub fn unloaded_below(inv: &Inventory, id: Uuid) -> Vec<Uuid> {
    let mut out = Vec::new();
    let mut stack = vec![id];
    let mut seen = HashSet::new();
    while let Some(folder) = stack.pop() {
        if !seen.insert(folder) {
            continue;
        }
        let Some(f) = inv.folders.get(&folder) else { continue };
        if f.state != FetchState::Fetched {
            if f.state != FetchState::Failed {
                out.push(folder);
            }
            continue;
        }
        stack.extend(f.children.iter().copied());
    }
    out
}

/// One entry to move.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Undo or a snapshot's layout: the entry goes back to a place it was,
    /// out of the Trash included; it is checked like a drag rather than
    /// by the hands-off rule, which still guards where it goes.
    #[serde(default)]
    pub restore: bool,
}

#[derive(Debug, Default, Clone)]
pub struct Plan {
    pub moves: Vec<Move>,
    /// How many moves are unpackers, counted by [`order`](Self::order).
    pub unpacker_count: usize,
    /// The destination groups (ranges of `moves`) in the preview's order,
    /// worked out once by [`order`](Self::order).
    pub groups: Vec<std::ops::Range<usize>>,
    /// Names that say nothing the sorter knows.
    pub unknown: Vec<String>,
    /// Entries that cannot move: worn, links, locked or system folders.
    pub kept: usize,
    /// Those entries by name, and why each stays: the preview shows them.
    pub kept_list: Vec<(String, Kept)>,
    /// Folders a Pre-Trash rule would read that are not loaded yet: the
    /// window asks for them and the preview fills in.
    pub unfetched: Vec<Uuid>,
}

impl Plan {
    /// The destination folders and how many entries each receives, by path;
    /// Pre-Trash first, so what would leave is seen before the rest.
    #[cfg(test)]
    pub fn groups(&self) -> Vec<(String, usize)> {
        self.grouped()
            .into_iter()
            .map(|(label, rows)| (label.to_owned(), rows.len()))
            .collect()
    }

    /// The moves by destination, in [`groups`](Self::groups)' order. The
    /// moves are sorted by destination when the plan is made, so each
    /// group is one slice: nothing is copied or counted per frame.
    pub fn grouped(&self) -> Vec<(&str, &[Move])> {
        // in the order worked out once by `order` (off the frame); a plan
        // not ordered yet is ordered here
        if !self.groups.is_empty() {
            return self
                .groups
                .iter()
                .map(|r| (self.moves[r.start].label.as_str(), &self.moves[r.clone()]))
                .collect();
        }
        let mut groups: Vec<(&str, &[Move])> = self
            .moves
            .chunk_by(|a, b| a.label == b.label)
            .map(|rows| (rows[0].label.as_str(), rows))
            .collect();
        groups.sort_by_cached_key(|(label, rows)| (rows[0].to != Dest::PreTrash, crate::natural_order::key(label)));
        groups
    }

    /// How many destination groups the preview has.
    pub fn group_count(&self) -> usize {
        if self.groups.is_empty() {
            self.moves.chunk_by(|a, b| a.label == b.label).count()
        } else {
            self.groups.len()
        }
    }

    /// What stays, by reason, each reason one slice of `kept_list` (in
    /// order once [`order`](Self::order) ran): nothing copied per frame.
    pub fn kept_by_reason(&self) -> Vec<(Kept, &[(String, Kept)])> {
        self.kept_list.chunk_by(|a, b| a.1 == b.1).map(|rows| (rows[0].1, rows)).collect()
    }

    /// Ready for the preview, once per plan (off the frame for a large
    /// one): what stays sorted by reason, keeping each reason's order.
    pub fn order(&mut self) {
        self.kept_list.sort_by_key(|(_, why)| *why);
        self.unpacker_count = self.unpackers();
        let mut groups: Vec<std::ops::Range<usize>> = Vec::new();
        let mut at = 0;
        for rows in self.moves.chunk_by(|a, b| a.label == b.label) {
            groups.push(at..at + rows.len());
            at += rows.len();
        }
        groups.sort_by_cached_key(|r| {
            let first = &self.moves[r.start];
            (first.to != Dest::PreTrash, crate::natural_order::key(&first.label))
        });
        self.groups = groups;
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
        .filter(|f| !f.library && f.info.parent == parent && f.info.type_default == -1 && same_name(&f.info.name, &name))
        .map(|f| f.info.id)
        .min()
}

/// Kept for the callers that ask about the Trash and Marketplace Listings:
/// both are hands-off, like every default folder but Objects.
pub fn untouchable(inv: &Inventory, id: Uuid) -> bool {
    hands_off(inv, id) || actions::in_type(inv, id, 14) || actions::in_type(inv, id, 53)
}

/// Whether the sorter can run on `source`: My Inventory, Objects, or one of
/// the user's own folders, in an open place ([`folder_open`]).
pub fn source_allowed(inv: &Inventory, source: Uuid) -> bool {
    folder_open(inv, source) && actions::mutable(inv, source)
}

/// A product's folder: an unpacked delivery, a fatpack with its HUD and
/// card. It is never opened: it moves whole or stays. It is a folder of the
/// user's (not one Auto-sort made) whose name reads as a product or a store,
/// or that holds no folder of its own.
///
/// The sorter's folder names (`reserved`) are worked out once by the caller,
/// for loops over thousands of folders.
fn product_folder_in(inv: &Inventory, folder: Uuid, reserved: &HashSet<String>) -> bool {
    let Some(f) = inv.folders.get(&folder) else { return false };
    let name = f.info.name.trim();
    if f.info.type_default != -1 || reserved.contains(&name.to_lowercase()) {
        return false;
    }
    let has_folders = f.children.iter().any(|c| inv.folders.contains_key(c));
    // a store before a dash (`Clothes - Winter`) is too weak a sign for a
    // folder of the user's: only a tag (`[Store]`, `*Store*`) makes it one
    let store_named = store(name).is_some();
    let tagged = store_named && VOCABULARY.store_tags.enclosed.iter().any(|(open, _)| name.starts_with(*open));
    let named = target(name, -1, 0, None, &[], false).is_some();
    if has_folders {
        // a folder of folders is the user's own (`My Dresses`, `Reborn
        // Stuff`, `Party Outfits`) unless it reads as a store's product: a
        // tag, a store and what it is (`Velmora - Hex Dress`), or what it
        // is with its versions inside (`Hex Dress` holding `Maitreya`,
        // `Reborn`, `Hex Dress Legacy`, `Squish`)
        tagged || store_named && named || named && versions_only(inv, f, name)
    } else {
        // a folder of items alone is a product when its name says what or
        // whose it is (`Hex Dress`, `[Velmora] Hex`); `Stuff`, `2024` or
        // `Hunt Gifts` are opened
        tagged || named || store_named
    }
}

/// Every subfolder of `f` is a version of the product named `name`: a body,
/// a Squish or Waifus variant, or the product's name again.
fn versions_only(inv: &Inventory, f: &super::Folder, name: &str) -> bool {
    let product = name.to_lowercase();
    f.children.iter().filter_map(|c| inv.folders.get(c)).all(|sub| {
        let sub_name = sub.info.name.trim();
        let read = classify(sub_name, -1, 0);
        read.body.is_some() || read.squish || read.waifus || sub_name.to_lowercase().contains(&product)
    })
}

/// How deep a whole-inventory sort opens the user's own folders of folders
/// (`Clothes/Winter/…`): below that, a folder moves whole or stays.
const OPEN_DEPTH: usize = 4;

/// What a whole-inventory sort reads, in an open place only: the loose
/// items of My Inventory, the contents of Objects, and the contents of the
/// user's own folders (not excluded, not the sort folder nor Pre-Trash),
/// their folders of folders a few levels down. A product's folder moves
/// whole, never opened. No other default folder is read (see
/// [`folder_open`]).
#[cfg(test)]
pub fn whole_sources(inv: &Inventory, prefs: &Preferences) -> Vec<Uuid> {
    let places = Places::of(inv, prefs);
    whole_sources_in(inv, prefs, &places)
}

fn whole_sources_in(inv: &Inventory, prefs: &Preferences, places: &Places) -> Vec<Uuid> {
    let mut sources = vec![inv.root];
    // the root's open folders, then the user's own folders of folders below
    // them (`Clothes/Winter`): each is opened, its products move whole. A
    // folder not loaded yet is read first (it is judged once its contents
    // are known).
    let mut level: Vec<Uuid> = vec![inv.root];
    for depth in 0..=OPEN_DEPTH {
        let mut next = Vec::new();
        for parent in &level {
            let Some(folder) = inv.folders.get(parent) else { continue };
            for id in folder.children.iter().copied() {
                let Some(f) = inv.folders.get(&id) else { continue };
                let open = Some(id) != places.base
                    && Some(id) != places.waiting
                    && !places.pre_trash.contains(&id)
                    && folder_open(inv, id)
                    && !prefs.excludes(inv, id)
                    && (depth == 0 || f.info.type_default == -1 && !places.reserved.contains(&f.info.name.trim().to_lowercase()));
                let own = f.info.type_default != -1 || f.state != FetchState::Fetched || !product_folder_in(inv, id, &places.reserved);
                if open && own {
                    sources.push(id);
                    // (Objects and an unlocked default folder too: their
                    // folders of folders are the user's own)
                    if f.state == FetchState::Fetched {
                        next.push(id);
                    }
                }
            }
        }
        if next.is_empty() || depth == OPEN_DEPTH {
            break;
        }
        level = next;
    }
    sources
}

/// What every folder of a plan is compared with, worked out once: the sort
/// folder, the Pre-Trash folder and the names of the folders Auto-sort
/// makes (a scan over 40,000 folders must not look them up for each one).
struct Places {
    base: Option<Uuid>,
    waiting: Option<Uuid>,
    /// Every Pre-Trash folder, other languages' included.
    pre_trash: HashSet<Uuid>,
    reserved: HashSet<String>,
}

impl Places {
    fn of(inv: &Inventory, prefs: &Preferences) -> Self {
        Self {
            base: base_folder(inv, prefs),
            waiting: pre_trash(inv),
            pre_trash: pre_trash_folders(inv),
            reserved: sorter_folder_names(prefs),
        }
    }
}

/// The plan of a whole-inventory sort (see [`whole_sources`]). Its
/// Pre-Trash rules read all of My Inventory's open places, at any depth
/// ([`pre_trash_scan`]). The default and viewer folders at the root are
/// listed as kept, so the preview names what is never touched.
pub fn plan_whole(inv: &Inventory, prefs: &Preferences, protected: &HashSet<Uuid>, worn: &HashSet<Uuid>) -> Plan {
    let places = Places::of(inv, prefs);
    let sources = whole_sources_in(inv, prefs, &places);
    let mut entries = Vec::new();
    for source in &sources {
        // the root's own folders are sources, products or hands-off: only
        // its loose items are entries
        let folders = prefs.folders && *source != inv.root;
        entries.extend(contents_in(inv, *source, folders, &places));
    }
    // the sources not loaded yet are asked for as the plan arrives (the
    // window only reads the root's folders on the frame)
    let unread: Vec<Uuid> = sources
        .iter()
        .copied()
        .filter(|id| inv.folders.get(id).is_some_and(|f| f.state != FetchState::Fetched))
        .collect();
    let sources: HashSet<Uuid> = sources.into_iter().collect();
    // a folder opened as a source is never moved as an entry of its parent
    entries.retain(|id| !sources.contains(id));
    // a product's folder at the root moves whole
    if prefs.folders
        && let Some(root) = inv.folders.get(&inv.root)
    {
        entries.extend(root.children.iter().copied().filter(|id| {
            !sources.contains(id)
                && Some(*id) != places.base
                && Some(*id) != places.waiting
                && folder_open(inv, *id)
                && product_folder_in(inv, *id, &places.reserved)
        }));
    }
    let scan = pre_trash_scan(inv, prefs, inv.root, protected, worn);
    let mut plan = plan_with(inv, entries, scan.found, prefs, protected, worn, true);
    // every list of folders to load: the plan's own, the scan's, the
    // sources'
    plan.unfetched.extend(scan.unfetched);
    plan.unfetched.extend(unread);
    plan.unfetched.sort_unstable();
    plan.unfetched.dedup();
    if let Some(root) = inv.folders.get(&inv.root) {
        for id in &root.children {
            if let Some(f) = inv.folders.get(id)
                && hands_off(inv, *id)
                && !folder_open(inv, *id)
            {
                plan.kept += 1;
                plan.kept_list.push((f.info.name.clone(), Kept::Protected));
            }
        }
    }
    plan
}
/// A folder name Auto-sort creates or looks up, as the inventory keeps it.
/// Inventory names are ASCII (LLInventoryObject::correctInventoryName,
/// indra/llinventory/llinventory.cpp; `operations::folder_name`): a name with
/// accents loses them (`Trié` → `Trie`, `Café` → `Cafe`) instead of having
/// them replaced by spaces, and a name that keeps no letter (Japanese) takes
/// `fallback`. Both the lookup and the creation use this name, so a run
/// finds the folders it made and never builds a second tree.
pub fn inventory_name(name: &str, fallback: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            c if c.is_ascii() => out.push(c),
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => out.push('a'),
            'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => out.push('A'),
            'ç' => out.push('c'),
            'Ç' => out.push('C'),
            'è' | 'é' | 'ê' | 'ë' => out.push('e'),
            'È' | 'É' | 'Ê' | 'Ë' => out.push('E'),
            'ì' | 'í' | 'î' | 'ï' => out.push('i'),
            'Ì' | 'Í' | 'Î' | 'Ï' => out.push('I'),
            'ñ' => out.push('n'),
            'Ñ' => out.push('N'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => out.push('o'),
            'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' => out.push('O'),
            'ù' | 'ú' | 'û' | 'ü' => out.push('u'),
            'Ù' | 'Ú' | 'Û' | 'Ü' => out.push('U'),
            'ý' | 'ÿ' => out.push('y'),
            'Ý' => out.push('Y'),
            'œ' => out.push_str("oe"),
            'Œ' => out.push_str("OE"),
            'æ' => out.push_str("ae"),
            'Æ' => out.push_str("AE"),
            'ß' => out.push_str("ss"),
            '’' | '‘' => out.push('\''),
            '–' | '—' => out.push('-'),
            ' ' | '\u{3000}' => out.push(' '),
            // a script with no ASCII spelling: the whole name falls back
            _ => return aurora_net::inventory::operations::folder_name(fallback),
        }
    }
    let name = aurora_net::inventory::operations::folder_name(&out);
    if name.chars().any(|c| c.is_ascii_alphanumeric()) {
        name
    } else {
        aurora_net::inventory::operations::folder_name(fallback)
    }
}

/// A category's folder as the inventory names it, English when the
/// language's name has no ASCII spelling.
pub fn category_folder(category: Category) -> String {
    let key = category.key().replace('-', " ");
    let mut english = key[..1].to_uppercase();
    english.push_str(&key[1..]);
    inventory_name(category.folder(), &english)
}

/// The default folder whose contents Auto-sort may read: Objects, where
/// deliveries land. Every other default folder is hands-off.
const OPEN_DEFAULT: i32 = 6;

/// A folder Auto-sort may read inside, move entries out of, or put entries
/// into: My Inventory itself (its loose items), Objects, and the user's own
/// folders, not in the Library. One shared rule for every path (sort, whole
/// inventory, right-click, Pre-Trash rules, Clean up, empty folders, the
/// Trash steps), checked when planning and again before each batch: no
/// setting, rule or imported text can reach past it.
fn open_container(inv: &Inventory, folder: &super::Folder) -> bool {
    !folder.library
        && (folder.info.id == inv.root
            || matches!(folder.info.type_default, -1 | OPEN_DEFAULT)
            || (unlockable(folder.info.type_default) || folder.info.type_default == LOST_AND_FOUND)
                && inv.sorter_unlocked.contains(&folder.info.id))
}

/// The default folders the user may allow Auto-sort into, one by one, with
/// a warning: Textures, Sounds, Calling Cards, Landmarks, Clothing,
/// Notecards, Scripts, Body Parts, Photo Album, Animations, Gestures,
/// Settings, Materials. Lost and Found opens by its own switch only
/// (`Preferences::include_lost_found`). Never: the Trash, Marketplace
/// Listings and its folders, the Current Outfit, My Outfits and the
/// outfits, the Library, Received Items, Favorites, the viewer folders.
pub fn unlockable(kind: i32) -> bool {
    matches!(kind, 0 | 1 | 2 | 3 | 5 | 7 | 10 | 13 | 15 | 20 | 21 | 56 | 57)
}

/// The move has happened: the entry is in the folder `m` sends it to
/// (its path resolved below the sort folder `base`), not where it was. A
/// batch the server applied late, or in part, is known by this.
pub fn arrived(inv: &Inventory, base: &str, m: &Move) -> bool {
    let Some(parent) = actions::parent(inv, m.id) else { return false };
    if parent == m.from {
        return false;
    }
    match &m.to {
        Dest::Folder(target) => parent == *target,
        Dest::PreTrash => Some(parent) == pre_trash(inv),
        Dest::Path(path) => {
            let mut at = inv.root;
            for part in std::iter::once(base).chain(path.iter().map(String::as_str)) {
                match child(inv, at, part) {
                    Some(id) => at = id,
                    None => return false,
                }
            }
            parent == at
        }
    }
}

/// Show an entry in `parent` locally, as the inventory does when a move is
/// sent (`NetEvent::InventoryMoved`): a refused batch's entry goes back
/// where it was until the server says otherwise.
pub fn show_in(inv: &mut Inventory, id: Uuid, folder: bool, parent: Uuid) {
    if actions::parent(inv, id) == Some(parent) || !inv.folders.contains_key(&parent) {
        return;
    }
    if folder {
        if let Some(f) = inv.folders.get(&id) {
            let mut moved = f.info.clone();
            moved.parent = parent;
            inv.server_updated(vec![moved], Vec::new());
        }
    } else if let Some(item) = inv.items.get(&id) {
        let mut moved = item.clone();
        moved.parent = parent;
        inv.server_updated(Vec::new(), vec![moved]);
    }
}

/// Lost and Found, semi-protected: closed unless its switch is on.
const LOST_AND_FOUND: i32 = 16;

/// The default folders of My Inventory's root the user may unlock.
pub fn unlockable_folders(inv: &Inventory) -> Vec<Uuid> {
    inv.folders
        .get(&inv.root)
        .map(|root| {
            root.children
                .iter()
                .copied()
                .filter(|id| inv.folders.get(id).is_some_and(|f| !f.library && unlockable(f.info.type_default)))
                .collect()
        })
        .unwrap_or_default()
}

/// The viewer folders at the root, never touched and never unlockable:
/// RLVa reads `#RLV` (RlvInventory::getSharedRootID), the AO and the
/// scripting bridge live in `#Firestorm` and `#Aurora`, and the AO reads
/// another viewer's `Animation Overrides` (ao::inventory::ALCHEMY_FOLDER).
/// Any other folder whose name starts with `#` is an ordinary folder of the
/// user's.
pub const VIEWER_FOLDERS: [&str; 4] = ["#RLV", "#Firestorm", "#Aurora", crate::ao::inventory::ALCHEMY_FOLDER];

fn viewer_folder(inv: &Inventory, folder: &super::Folder) -> bool {
    folder.info.parent == inv.root && VIEWER_FOLDERS.iter().any(|name| folder.info.name.trim().eq_ignore_ascii_case(name))
}

/// `folder` and every folder above it are open: nothing in it is hands-off
/// by place. Hands-off at any depth: every default folder but Objects
/// (Clothing, Body Parts, Landmarks, Calling Cards, Favorites, Materials,
/// Settings, Lost and Found, Photo Album…), My Outfits and each outfit, the
/// Current Outfit, the Trash, Marketplace Listings and its stock and version
/// folders, the old Merchant Outbox, Received Items, the Library, and the
/// `#` viewer folders.
pub fn folder_open(inv: &Inventory, folder: Uuid) -> bool {
    // the inventory's root not known yet: nothing is open
    if inv.root.is_nil() {
        return false;
    }
    for id in actions::ancestors(inv, folder) {
        if id == inv.root {
            return true;
        }
        let Some(f) = inv.folders.get(&id) else { return false };
        if !open_container(inv, f) || viewer_folder(inv, f) {
            return false;
        }
    }
    // not under My Inventory (the Library, an orphan, a cycle)
    false
}

/// An entry (item or folder) Auto-sort must never move, trash or open: a
/// default folder itself (it never moves, Objects included), a `#` viewer
/// folder, or anything in a hands-off folder ([`folder_open`]).
pub fn hands_off(inv: &Inventory, id: Uuid) -> bool {
    if let Some(f) = inv.folders.get(&id)
        && (f.info.type_default != -1 || f.library || viewer_folder(inv, f) || id == inv.root)
    {
        return true;
    }
    actions::parent(inv, id).is_none_or(|parent| !folder_open(inv, parent))
}

/// Re-checked just before an entry goes to the Trash (Clean up, Empty
/// Pre-Trash): it is still where the confirmed list saw it, nowhere
/// hands-off, not excluded, and movable (nothing worn in it, no link, no
/// locked folder above or below it). What changed since is skipped.
pub fn trash_ready(
    inv: &Inventory,
    id: Uuid,
    from: Uuid,
    prefs: &Preferences,
    protected: &HashSet<Uuid>,
    holds_worn: &HashSet<Uuid>,
) -> bool {
    actions::parent(inv, id) == Some(from)
        && !hands_off(inv, id)
        && !prefs.excludes(inv, id)
        && !holds_closed(inv, id, &prefs.excluded)
        && movable(inv, id, protected, holds_worn)
}

/// An entry Undo or a snapshot may put back: movable by a drag (out of the
/// Trash or Pre-Trash too), nothing worn in it, no link, no locked folder
/// above or below it, never a default or viewer folder itself.
pub fn restorable(inv: &Inventory, id: Uuid, protected: &HashSet<Uuid>, holds_worn: &HashSet<Uuid>) -> bool {
    let own_folder = inv
        .folders
        .get(&id)
        .is_none_or(|f| f.info.type_default == -1 && !viewer_folder(inv, f));
    // it leaves an open place, the Trash or Pre-Trash only: never an outfit,
    // the Current Outfit or any other hands-off folder
    let trash = actions::system(inv, 14);
    let leaves = actions::parent(inv, id).is_some_and(|p| folder_open(inv, p) || trash.is_some_and(|t| actions::under(inv, p, t)));
    own_folder
        && leaves
        && !inv.folders.get(&id).is_some_and(|f| f.library)
        && !actions::library(inv, id)
        && actions::relocatable(inv, id, protected)
        && !holds_worn.contains(&id)
        && inv.items.get(&id).is_none_or(|it| !matches!(it.asset_type, 24 | 25))
        && !actions::ancestors(inv, id).any(|a| protected.contains(&a))
        && !protected.iter().any(|p| actions::under(inv, *p, id))
        // an outfit or another typed folder inside never travels with it
        && !holds_closed(inv, id, &[])
}

/// The preview of saved moves (Undo, a snapshot's layout, a run to resume):
/// each re-checked against the inventory now, what can no longer go is
/// listed as kept.
pub fn plan_saved(inv: &Inventory, moves: &[Move], prefs: &Preferences, protected: &HashSet<Uuid>, worn: &HashSet<Uuid>) -> Plan {
    let holds_worn = actions::worn_ancestors(inv, worn);
    let mut plan = Plan::default();
    for m in moves {
        let here = actions::parent(inv, m.id) == Some(m.from);
        // a folder whose tree is still loading: kept for now, asked for
        // (only one that could move: never excluded, locked, worn or
        // protected, nor a tree loaded only to be skipped)
        if here
            && m.folder
            && (m.restore || !prefs.excludes(inv, m.id))
            && actions::relocatable(inv, m.id, protected)
            && !holds_worn.contains(&m.id)
            && match m.to {
                Dest::Folder(target) if m.restore => restore_target_allowed(inv, target),
                Dest::Folder(target) => target_allowed(inv, target),
                Dest::Path(_) | Dest::PreTrash => true,
            }
        {
            let unloaded = unloaded_below(inv, m.id);
            if !unloaded.is_empty() {
                // the run waits for its tree before it goes (step), so it is
                // never left out of a Resume or an Undo
                plan.unfetched.extend(unloaded);
                plan.moves.push(m.clone());
                continue;
            }
        }
        // a folder excluded since (or an entry in one) stays, like any sort's
        if here && !m.restore && (prefs.excludes(inv, m.id) || holds_closed(inv, m.id, &prefs.excluded)) {
            plan.kept += 1;
            plan.kept_list.push((m.name.clone(), Kept::Excluded));
            continue;
        }
        let ok =
            here && if m.restore {
                restorable(inv, m.id, protected, &holds_worn)
            } else {
                movable(inv, m.id, protected, &holds_worn)
            } && match m.to {
                Dest::Folder(target) if m.restore => restore_target_allowed(inv, target),
                Dest::Folder(target) => target_allowed(inv, target),
                Dest::Path(_) | Dest::PreTrash => true,
            };
        if ok {
            plan.moves.push(m.clone());
        } else if here {
            plan.kept += 1;
            plan.kept_list.push((m.name.clone(), Kept::Protected));
        }
    }
    plan.moves.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.name.cmp(&b.name)));
    plan
}

/// Who an item comes from: its creator (Properties' Creator), or, for an
/// object whose parts or contents have several creators (the simulator
/// sends none then: "unknown / multiple", LLPermissions::accumulate), the
/// account it came from (its last owner: the store's vendor or Marketplace
/// account), never its owner.
fn maker(inv: &Inventory, it: &InvItem) -> Option<Uuid> {
    if !it.creator.is_nil() {
        return Some(it.creator);
    }
    let own = it.last_owner == it.owner || it.last_owner == inv.owner;
    (!it.last_owner.is_nil() && !own).then_some(it.last_owner)
}

/// Who made an entry: an item's [`maker`], or for a folder (a fatpack) the
/// maker of most of its items.
pub fn creator_of(inv: &Inventory, id: Uuid) -> Option<Uuid> {
    if let Some(it) = inv.items.get(&id) {
        return maker(inv, it);
    }
    let folder = inv.folders.get(&id)?;
    let mut counts: HashMap<Uuid, usize> = HashMap::new();
    for it in folder.items.iter().filter_map(|i| inv.items.get(i)) {
        if let Some(who) = maker(inv, it) {
            *counts.entry(who).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(creator, count)| (*count, std::cmp::Reverse(*creator)))
        .map(|(creator, _)| creator)
}

/// What a plan reads of the inventory, copied for a background planner:
/// the folders, the items, the roots and what decides the hands-off and
/// held places (Marketplace holds, unlocked default folders, broken links).
/// The fetch queue, caches and repair state stay with the live inventory.
pub fn snapshot(inv: &Inventory) -> Inventory {
    Inventory {
        owner: inv.owner,
        root: inv.root,
        lib_root: inv.lib_root,
        lib_owner: inv.lib_owner,
        folders: inv.folders.clone(),
        items: inv.items.clone(),
        generation: inv.generation,
        missing_items: inv.missing_items.clone(),
        too_big: inv.too_big.clone(),
        listing_holds: inv.listing_holds.clone(),
        sorter_unlocked: inv.sorter_unlocked.clone(),
        ..Default::default()
    }
}

/// The creator hint the right-click "Sort by this creator" adds for `id`:
/// its creator (a folder's majority creator), the store its name gives and
/// the category its name gives (Furniture when it says nothing; the user
/// picks another in Options). None when nobody is known to have made it.
pub fn creator_hint(inv: &Inventory, id: Uuid, creator_name: &str) -> Option<StoreHint> {
    let creator = creator_of(inv, id)?;
    let (name, asset_type, flags) = match (inv.folders.get(&id), inv.items.get(&id)) {
        (Some(f), _) => (f.info.name.as_str(), -1, 0),
        (None, Some(it)) => (it.name.as_str(), it.asset_type, it.flags),
        _ => return None,
    };
    Some(StoreHint {
        store: store(name).unwrap_or_default(),
        category: classify(name, asset_type, flags).category.unwrap_or(StoreHint::default().category),
        creator: Some(creator),
        creator_name: creator_name.chars().take(64).collect(),
    })
}

/// Why an entry the preview lists stays where it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kept {
    /// Worn, or a folder holding something worn.
    Worn,
    /// A link: it belongs to its folder (an outfit's).
    Link,
    /// A locked folder, or inside one.
    Locked,
    /// In a folder the user excluded.
    Excluded,
    /// A system folder, the Trash, Marketplace Listings, the Library, the
    /// Current Outfit, a viewer folder (#RLV…): what a drag may not move.
    Protected,
    /// A folder whose subfolders are not loaded yet: it moves whole once
    /// they are (the preview asks for them).
    Loading,
}

impl Kept {
    pub fn label(self) -> &'static str {
        match self {
            Kept::Worn => aurora_i18n::tr!("inventory-sorter-kept-worn"),
            Kept::Link => aurora_i18n::tr!("inventory-sorter-kept-link"),
            Kept::Locked => aurora_i18n::tr!("inventory-sorter-kept-locked"),
            Kept::Excluded => aurora_i18n::tr!("inventory-sorter-kept-excluded"),
            Kept::Protected => aurora_i18n::tr!("inventory-sorter-kept-protected"),
            Kept::Loading => aurora_i18n::tr!("inventory-sorter-kept-loading"),
        }
    }
}

fn kept_reason(inv: &Inventory, id: Uuid, prefs: &Preferences, protected: &HashSet<Uuid>, holds_worn: &HashSet<Uuid>) -> Kept {
    if prefs.excludes(inv, id) {
        Kept::Excluded
    } else if holds_worn.contains(&id) {
        Kept::Worn
    } else if inv.items.get(&id).is_some_and(|it| matches!(it.asset_type, 24 | 25)) {
        Kept::Link
    } else if actions::ancestors(inv, id).any(|a| protected.contains(&a)) || protected.contains(&id) {
        Kept::Locked
    } else {
        Kept::Protected
    }
}

/// Can `id` leave its folder for the sorted ones? The rules of a drag
/// (`relocatable`), nothing worn and no link (an outfit's link belongs to
/// its folder).
fn movable(inv: &Inventory, id: Uuid, protected: &HashSet<Uuid>, holds_worn: &HashSet<Uuid>) -> bool {
    actions::relocatable(inv, id, protected)
        && !untouchable(inv, id)
        && !holds_worn.contains(&id)
        && inv.items.get(&id).is_none_or(|it| !matches!(it.asset_type, 24 | 25))
        // a locked folder keeps all it holds, and a folder holding a locked
        // one never moves whole with it
        && !actions::ancestors(inv, id).any(|a| protected.contains(&a))
        && (!inv.folders.contains_key(&id) || !protected.iter().any(|p| actions::under(inv, *p, id)))
        // an outfit, a stock folder or any other typed folder inside never
        // travels with it
        && !holds_closed(inv, id, &[])
}

/// The names the sorter gives its own folders: a folder already named like
/// one is a sorted folder, never moved into another.
fn sorter_folder_names(prefs: &Preferences) -> HashSet<String> {
    let mut names: HashSet<String> = VOCABULARY.bodies.iter().map(|b| b.body.to_lowercase()).collect();
    names.insert(Body::Several.folder().to_lowercase());
    names.insert(prefs.base_name().to_lowercase());
    names.insert(pre_trash_name().to_lowercase());
    for (waifus, squish) in [(true, false), (false, true), (true, true)] {
        names.insert(variant_folder(waifus, squish).to_lowercase());
    }
    names.insert(stores_folder().to_lowercase());
    for category in Category::ALL {
        names.insert(category_folder(category).to_lowercase());
    }
    for rule in prefs.rules.iter().filter(|r| r.target.is_none()) {
        names.extend(split_path(&rule.folder).into_iter().map(|p| p.to_lowercase()));
    }
    names
}

/// The moves of the direct contents of `source`. A folder goes as a whole
/// (`prefs.folders`), classified by its own name. Entries already in the
/// folder they would go to are left out. The Pre-Trash rules read further:
/// the whole of `source` at any depth, and the sort folder's tree
/// ([`pre_trash_scan`]).
pub fn plan(inv: &Inventory, source: Uuid, prefs: &Preferences, protected: &HashSet<Uuid>, worn: &HashSet<Uuid>) -> Plan {
    let entries = contents(inv, source, prefs, prefs.folders);
    let scan = pre_trash_scan(inv, prefs, source, protected, worn);
    let mut plan = plan_with(inv, entries, scan.found, prefs, protected, worn, true);
    // both lists of folders to load: the plan's own and the scan's
    plan.unfetched.extend(scan.unfetched);
    plan.unfetched.sort_unstable();
    plan.unfetched.dedup();
    plan
}

/// A usable rule sends to Pre-Trash.
pub fn has_pre_trash_rule(prefs: &Preferences) -> bool {
    prefs.rules.iter().any(|r| r.pre_trash && r.usable())
}

/// What the Pre-Trash rules take beyond the direct contents of a sort.
#[derive(Debug, Default, Clone)]
pub struct PreTrashScan {
    /// Items, and folders that go whole.
    pub found: Vec<Uuid>,
    /// Folders not loaded yet, skipped for now.
    pub unfetched: Vec<Uuid>,
}

/// System folders a Pre-Trash rule never reads: Favorites, the Current
/// Outfit, an outfit and My Outfits (the Trash and Marketplace Listings are
/// [`untouchable`], the Library is not the agent's).
const NEVER_READ: &[i32] = &[23, 46, 47, 48];

/// The rules that apply to an entry: in the sort folder's tree, a Pre-Trash
/// rule set to "Everywhere except Sorted" does not.
fn rules_for_place(rules: &[Rule], sorted: bool) -> Vec<&Rule> {
    rules.iter().filter(|r| !(sorted && r.pre_trash && r.skip_sorted)).collect()
}

fn first_rule<'a>(rules: &[&'a Rule], name_words: &[String], created: Option<i64>) -> Option<&'a Rule> {
    rules
        .iter()
        .copied()
        .find(|rule| rule.usable() && rule.matches(name_words, created))
}

/// What the Pre-Trash rules take, sorted or not: everything in `source` at
/// any depth, and the sort folder's tree unless every Pre-Trash rule leaves
/// it out. An item goes when its first usable rule is a Pre-Trash one; a
/// product's folder (a delivery, a fatpack) is never split: it goes whole
/// when a Pre-Trash rule names it, or when a dated one finds every item in
/// it (at any depth) acquired before its date, and otherwise stays with all
/// it holds. Only the folders Auto-sort made, the system folders, the
/// folders whose name reads as no product (the user's own arrangement) and
/// `source` itself are opened. Never read: the Trash, Marketplace Listings,
/// the Library, Favorites, outfits, the Current Outfit, the Pre-Trash folder,
/// excluded and locked folders; worn entries and links stay.
pub fn pre_trash_scan(inv: &Inventory, prefs: &Preferences, source: Uuid, protected: &HashSet<Uuid>, worn: &HashSet<Uuid>) -> PreTrashScan {
    let mut scan = PreTrashScan::default();
    if !has_pre_trash_rule(prefs) {
        return scan;
    }
    let base = base_folder(inv, prefs);
    let in_sorted = |id: Uuid| base.is_some_and(|b| id == b || actions::under(inv, id, b));
    let mut stack = vec![source];
    if let Some(base) = base
        && prefs.rules.iter().any(|r| r.pre_trash && r.usable() && !r.skip_sorted)
    {
        stack.push(base);
    }
    let reserved = sorter_folder_names(prefs);
    let holds_worn = actions::worn_ancestors(inv, worn);
    let pre_trash_at = pre_trash(inv);
    let waiting_all = pre_trash_folders(inv);
    let closed = |id: Uuid| {
        inv.folders
            .get(&id)
            .is_none_or(|f| f.library || NEVER_READ.contains(&f.info.type_default))
            || Some(id) == pre_trash_at
            || waiting_all.contains(&id)
            || protected.contains(&id)
            || prefs.excludes(inv, id)
            || !folder_open(inv, id)
    };
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) || closed(id) {
            continue;
        }
        let Some(folder) = inv.folders.get(&id) else { continue };
        if folder.state != FetchState::Fetched {
            scan.unfetched.push(id);
            continue;
        }
        let rules = rules_for_place(&prefs.rules, in_sorted(id));
        for item in &folder.items {
            if let Some(it) = inv.items.get(item)
                && movable(inv, *item, protected, &holds_worn)
                && first_rule(&rules, &words(&it.name), Some(it.created_at)).is_some_and(|r| r.pre_trash)
            {
                scan.found.push(*item);
            }
        }
        for sub in &folder.children {
            let Some(f) = inv.folders.get(sub) else { continue };
            if closed(*sub) {
                continue;
            }
            let name = f.info.name.trim();
            // the sort folder itself is never one entry to take whole
            let system = f.info.type_default != -1 || Some(*sub) == base;
            let made = reserved.contains(&name.to_lowercase())
                || prefs.stores && in_sorted(*sub) && store(name).is_none() && target(name, -1, 0, None, &[], false).is_none();
            if !system && !made && movable(inv, *sub, protected, &holds_worn) {
                let name_words = words(name);
                let rules = rules_for_place(&prefs.rules, in_sorted(*sub));
                if first_rule(&rules, &name_words, None).is_some_and(|rule| rule.pre_trash) {
                    scan.found.push(*sub);
                    continue;
                }
            }
            // a product's folder stays whole: its old items are not pulled out
            let product = !system && !made && product_folder_in(inv, *sub, &reserved);
            if !product {
                stack.push(*sub);
            }
        }
    }
    scan.unfetched.sort_unstable();
    scan.unfetched.dedup();
    scan
}

/// Every item in `folder`, at any depth, passes `keep`, and there is at
/// least one. A folder not loaded yet answers no (and is listed in
/// `unfetched`).
fn all_items(inv: &Inventory, folder: Uuid, unfetched: &mut Vec<Uuid>, keep: impl Fn(&InvItem) -> bool) -> bool {
    let mut stack = vec![folder];
    let mut seen = HashSet::new();
    let mut any = false;
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(f) = inv.folders.get(&id) else { return false };
        // a typed folder inside (an outfit…) keeps the whole folder out
        if id != folder && f.info.type_default != -1 {
            return false;
        }
        if f.state != FetchState::Fetched {
            unfetched.push(id);
            return false;
        }
        for item in &f.items {
            match inv.items.get(item) {
                Some(it) if keep(it) => any = true,
                _ => return false,
            }
        }
        stack.extend(f.children.iter().copied());
    }
    any
}

/// The item kinds Clean up filters on, by asset type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Kind {
    Objects,
    Clothing,
    BodyParts,
    Notecards,
    Landmarks,
    Textures,
    Scripts,
    Sounds,
    Animations,
    Gestures,
    Materials,
    CallingCards,
}

impl Kind {
    pub const ALL: [Kind; 12] = [
        Kind::Objects,
        Kind::Clothing,
        Kind::BodyParts,
        Kind::Notecards,
        Kind::Landmarks,
        Kind::Textures,
        Kind::Scripts,
        Kind::Sounds,
        Kind::Animations,
        Kind::Gestures,
        Kind::Materials,
        Kind::CallingCards,
    ];

    /// The kind of an asset type; links (24, 25) and the rest have none and
    /// are never cleaned up.
    pub fn of(asset_type: i32) -> Option<Kind> {
        Some(match asset_type {
            6 => Kind::Objects,
            5 => Kind::Clothing,
            13 => Kind::BodyParts,
            7 => Kind::Notecards,
            3 => Kind::Landmarks,
            0 => Kind::Textures,
            10 => Kind::Scripts,
            1 => Kind::Sounds,
            20 => Kind::Animations,
            21 => Kind::Gestures,
            56 | 57 => Kind::Materials,
            2 => Kind::CallingCards,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Objects => aurora_i18n::tr!("inventory-cleanup-kind-objects"),
            Kind::Clothing => aurora_i18n::tr!("inventory-cleanup-kind-clothing"),
            Kind::BodyParts => aurora_i18n::tr!("inventory-cleanup-kind-body-parts"),
            Kind::Notecards => aurora_i18n::tr!("inventory-cleanup-kind-notecards"),
            Kind::Landmarks => aurora_i18n::tr!("inventory-cleanup-kind-landmarks"),
            Kind::Textures => aurora_i18n::tr!("inventory-cleanup-kind-textures"),
            Kind::Scripts => aurora_i18n::tr!("inventory-cleanup-kind-scripts"),
            Kind::Sounds => aurora_i18n::tr!("inventory-cleanup-kind-sounds"),
            Kind::Animations => aurora_i18n::tr!("inventory-cleanup-kind-animations"),
            Kind::Gestures => aurora_i18n::tr!("inventory-cleanup-kind-gestures"),
            Kind::Materials => aurora_i18n::tr!("inventory-cleanup-kind-materials"),
            Kind::CallingCards => aurora_i18n::tr!("inventory-cleanup-kind-calling-cards"),
        }
    }
}

/// How old Clean up's entries are: a chip of the tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Age {
    #[default]
    OneYear,
    TwoYears,
    ThreeYears,
    FiveYears,
    /// The year and month chosen in the tab.
    Custom,
}

/// Clean up: everything acquired before a date, of the chosen kinds, sent to
/// Pre-Trash or the Trash. Saved with the inventory preferences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cleanup {
    pub age: Age,
    /// The custom date: acquired before the first day of `month` (1–12) of
    /// `year`.
    pub year: i32,
    pub month: u32,
    pub kinds: Vec<Kind>,
    /// All of My Inventory rather than the folder the window sorts.
    pub whole: bool,
    /// Read the sort folder's tree too.
    pub include_sorted: bool,
    /// Take the user's empty folders too (nothing in them at any depth).
    pub empty_folders: bool,
}

impl Default for Cleanup {
    fn default() -> Self {
        Self {
            age: Age::default(),
            year: 2020,
            month: 1,
            kinds: Kind::ALL.to_vec(),
            whole: true,
            include_sorted: true,
            empty_folders: false,
        }
    }
}

impl Cleanup {
    /// The cutoff in seconds since 1970, UTC: entries acquired strictly
    /// before it go. The chips count back from `now`'s day (29 February
    /// falls back to the 28th).
    pub fn cutoff(&self, now: i64) -> Option<i64> {
        use chrono::Datelike;
        let years = match self.age {
            Age::OneYear => 1,
            Age::TwoYears => 2,
            Age::ThreeYears => 3,
            Age::FiveYears => 5,
            Age::Custom => return date_start(&format!("{}-{:02}", self.year, self.month.clamp(1, 12))),
        };
        let today = chrono::DateTime::from_timestamp(now, 0)?.date_naive();
        let year = today.year() - years;
        let day = today
            .with_year(year)
            .or_else(|| chrono::NaiveDate::from_ymd_opt(year, today.month(), 28))?;
        Some(day.and_hms_opt(0, 0, 0)?.and_utc().timestamp())
    }

    fn takes(&self, it: &InvItem, cutoff: i64) -> bool {
        it.created_at > 0 && it.created_at < cutoff && Kind::of(it.asset_type).is_some_and(|k| self.kinds.contains(&k))
    }
}

/// One entry Clean up would take: an item, or a folder that goes whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupRow {
    pub id: Uuid,
    pub folder: bool,
    pub from: Uuid,
    pub name: String,
    /// The item's kind; a folder has none.
    pub kind: Option<Kind>,
    /// The acquired date; for a folder, its newest item's.
    pub created: i64,
    /// The items a folder holds, at any depth (1 for an item).
    pub items: usize,
}

#[derive(Debug, Default, Clone)]
pub struct CleanupPlan {
    /// In [`CleanupRow::group`] order.
    pub rows: Vec<CleanupRow>,
    /// The items all rows hold.
    pub items: usize,
    pub cutoff: i64,
    /// Folders not loaded yet: the window asks for them, the list fills in.
    pub unfetched: Vec<Uuid>,
}

impl CleanupRow {
    /// The list's group: folders with items (they go whole), empty
    /// folders, then each kind in the filter's order.
    pub fn group(&self) -> usize {
        match (self.folder, self.kind) {
            (true, _) if self.items > 0 => 0,
            (true, _) => 1,
            (false, Some(kind)) => 2 + Kind::ALL.iter().position(|k| *k == kind).unwrap_or(Kind::ALL.len()),
            (false, None) => 2 + Kind::ALL.len(),
        }
    }
}

impl CleanupPlan {
    /// The rows by group, each one slice of `rows`.
    pub fn groups(&self) -> impl Iterator<Item = &[CleanupRow]> {
        self.rows.chunk_by(|a, b| a.group() == b.group())
    }

    /// The moves to Pre-Trash of the rows not in `skipped`.
    pub fn moves(&self, skipped: &HashSet<Uuid>) -> Vec<Move> {
        self.rows
            .iter()
            .filter(|r| !skipped.contains(&r.id))
            .map(|r| Move {
                id: r.id,
                folder: r.folder,
                name: r.name.clone(),
                from: r.from,
                to: Dest::PreTrash,
                label: pre_trash_name(),
                unpacker: false,
                restore: false,
            })
            .collect()
    }
}

/// What Clean up takes from `source` (My Inventory for `cleanup.whole`), at
/// any depth: the items acquired before the cutoff and of the chosen kinds.
/// Folders are read like the Pre-Trash rules read them
/// ([`pre_trash_scan`]): a folder whose items all match goes whole and is
/// not opened; a product's folder (its name reads as a product: a delivery,
/// a fatpack) with one item that does not match stays whole, old items
/// included; the user's own folders, the system folders and the folders
/// Auto-sort made are opened and their matching items go one by one. Never
/// read: the Trash, Marketplace Listings, the Library, Favorites, outfits,
/// the Current Outfit, the Pre-Trash folder, excluded and locked folders,
/// and the sort folder's tree unless `include_sorted`; worn entries and
/// links stay.
pub fn plan_cleanup(
    inv: &Inventory,
    prefs: &Preferences,
    cleanup: &Cleanup,
    source: Uuid,
    now: i64,
    protected: &HashSet<Uuid>,
    worn: &HashSet<Uuid>,
) -> CleanupPlan {
    let mut plan = CleanupPlan::default();
    let Some(cutoff) = cleanup.cutoff(now) else { return plan };
    plan.cutoff = cutoff;
    if cleanup.kinds.is_empty() && !cleanup.empty_folders {
        return plan;
    }
    let source = if cleanup.whole { inv.root } else { source };
    let base = base_folder(inv, prefs);
    let reserved = sorter_folder_names(prefs);
    let holds_worn = actions::worn_ancestors(inv, worn);
    let pre_trash_at = pre_trash(inv);
    let waiting_all = pre_trash_folders(inv);
    let closed = |id: Uuid| {
        inv.folders
            .get(&id)
            .is_none_or(|f| f.library || NEVER_READ.contains(&f.info.type_default))
            || Some(id) == pre_trash_at
            || waiting_all.contains(&id)
            || protected.contains(&id)
            || prefs.excludes(inv, id)
            || !folder_open(inv, id)
            || !cleanup.include_sorted && base.is_some_and(|b| id == b || actions::under(inv, id, b))
    };
    let mut stack = vec![source];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) || closed(id) {
            continue;
        }
        let Some(folder) = inv.folders.get(&id) else { continue };
        if folder.state != FetchState::Fetched {
            plan.unfetched.push(id);
            continue;
        }
        for item in &folder.items {
            if let Some(it) = inv.items.get(item)
                && cleanup.takes(it, cutoff)
                && movable(inv, *item, protected, &holds_worn)
            {
                plan.rows.push(CleanupRow {
                    id: *item,
                    folder: false,
                    from: id,
                    name: it.name.clone(),
                    kind: Kind::of(it.asset_type),
                    created: it.created_at,
                    items: 1,
                });
            }
        }
        for sub in &folder.children {
            let Some(f) = inv.folders.get(sub) else { continue };
            if closed(*sub) {
                continue;
            }
            let name = f.info.name.trim();
            // the sort folder itself is never one entry to take whole
            let system = f.info.type_default != -1 || Some(*sub) == base;
            let made = reserved.contains(&name.to_lowercase());
            // an empty folder (nothing at any depth), the user's or one
            // Auto-sort made, never the sort folder itself
            if cleanup.empty_folders
                && !system
                && Some(*sub) != base
                && movable(inv, *sub, protected, &holds_worn)
                && !holds_closed(inv, *sub, &prefs.excluded)
                && empty_tree(inv, *sub, &mut plan.unfetched)
            {
                plan.rows.push(CleanupRow {
                    id: *sub,
                    folder: true,
                    from: id,
                    name: f.info.name.clone(),
                    kind: None,
                    created: 0,
                    items: 0,
                });
                continue;
            }
            if !system
                && !made
                && movable(inv, *sub, protected, &holds_worn)
                // an excluded folder inside keeps the whole folder out
                && !holds_closed(inv, *sub, &prefs.excluded)
                && all_items(inv, *sub, &mut plan.unfetched, |it| cleanup.takes(it, cutoff))
            {
                let (count, newest) = folder_items(inv, *sub);
                plan.rows.push(CleanupRow {
                    id: *sub,
                    folder: true,
                    from: id,
                    name: f.info.name.clone(),
                    kind: None,
                    created: newest,
                    items: count,
                });
                continue;
            }
            // a product's folder stays whole: its old items are not pulled out
            let product = !system && !made && product_folder_in(inv, *sub, &reserved);
            if !product {
                stack.push(*sub);
            }
        }
    }
    plan.unfetched.sort_unstable();
    plan.unfetched.dedup();
    // by group (the list's order, so each group is one slice, never
    // filtered again per frame), then oldest first
    plan.rows.sort_by(|a, b| {
        a.group()
            .cmp(&b.group())
            .then_with(|| a.created.cmp(&b.created))
            .then_with(|| a.name.cmp(&b.name))
    });
    plan.items = plan.rows.iter().map(|r| r.items).sum();
    plan
}

/// `folder` holds no item at any depth (only empty folders, or nothing). A
/// folder not loaded yet answers no (and is listed in `unfetched`).
fn empty_tree(inv: &Inventory, folder: Uuid, unfetched: &mut Vec<Uuid>) -> bool {
    let mut stack = vec![folder];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(f) = inv.folders.get(&id) else { return false };
        if f.state != FetchState::Fetched {
            unfetched.push(id);
            return false;
        }
        if !f.items.is_empty()
            || f.children
                .iter()
                .any(|c| inv.folders.get(c).is_some_and(|c| c.info.type_default != -1))
        {
            return false;
        }
        stack.extend(f.children.iter().copied());
    }
    true
}

/// How many items `folder` holds at any depth, and the newest acquired date.
fn folder_items(inv: &Inventory, folder: Uuid) -> (usize, i64) {
    let mut stack = vec![folder];
    let mut seen = HashSet::new();
    let (mut count, mut newest) = (0, 0);
    while let Some(id) = stack.pop() {
        let Some(f) = inv.folders.get(&id).filter(|_| seen.insert(id)) else {
            continue;
        };
        for it in f.items.iter().filter_map(|i| inv.items.get(i)) {
            count += 1;
            newest = newest.max(it.created_at);
        }
        stack.extend(f.children.iter().copied());
    }
    (count, newest)
}

/// The direct contents of `source` a sort reads: its items, and its folders
/// when `folders`, less the sorted folders themselves.
fn contents(inv: &Inventory, source: Uuid, prefs: &Preferences, folders: bool) -> Vec<Uuid> {
    contents_in(inv, source, folders, &Places::of(inv, prefs))
}

fn contents_in(inv: &Inventory, source: Uuid, folders: bool, places: &Places) -> Vec<Uuid> {
    let Some(folder) = inv.folders.get(&source) else {
        return Vec::new();
    };
    let (base, reserved) = (places.base, &places.reserved);
    let mut entries: Vec<Uuid> = folder.items.clone();
    if source == inv.root {
        // My Inventory's own folders are never moved by a sort of it, but a
        // product's folder lying loose at the root goes whole
        if folders {
            entries.extend(
                folder
                    .children
                    .iter()
                    .copied()
                    .filter(|id| Some(*id) != base && Some(*id) != places.waiting && product_folder_in(inv, *id, reserved)),
            );
        }
        return entries;
    }
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
    plan_with(inv, entries.to_vec(), Vec::new(), prefs, protected, worn, false)
}

/// The sort's type filter takes `asset_type` (a folder, or an item of no
/// known kind, always).
fn kind_wanted(prefs: &Preferences, asset_type: i32) -> bool {
    asset_type == -1 || Kind::of(asset_type).is_none_or(|k| prefs.kinds.contains(&k))
}

/// The moves of `entries`, and of `pre_trashed` (what [`pre_trash_scan`]
/// found), which go to Pre-Trash whatever their name says.
fn plan_with(
    inv: &Inventory,
    mut entries: Vec<Uuid>,
    pre_trashed: Vec<Uuid>,
    prefs: &Preferences,
    protected: &HashSet<Uuid>,
    worn: &HashSet<Uuid>,
    filtered: bool,
) -> Plan {
    let mut plan = Plan::default();
    let forced: HashSet<Uuid> = pre_trashed.iter().copied().collect();
    let known: HashSet<Uuid> = entries.iter().copied().collect();
    entries.extend(pre_trashed.into_iter().filter(|id| !known.contains(id)));
    let holds_worn = actions::worn_ancestors(inv, worn);
    let base = base_folder(inv, prefs);
    // in the sort folder's tree, the "Everywhere except Sorted" rules are
    // left out
    let sorted_rules: Vec<Rule> = prefs.rules.iter().filter(|r| !(r.pre_trash && r.skip_sorted)).cloned().collect();
    let waiting_all = pre_trash_folders(inv);
    // only read who made an entry when a creator hint can use it
    let by_creator = prefs.store_hints.iter().any(|h| h.creator.is_some());
    let mut candidates = Vec::new();
    for id in entries.iter().copied() {
        let (name, asset_type, flags, created, is_folder) = match (inv.folders.get(&id), inv.items.get(&id)) {
            (Some(f), _) => (f.info.name.clone(), -1, 0, None, true),
            (None, Some(it)) => (it.name.clone(), it.asset_type, it.flags, Some(it.created_at), false),
            _ => continue,
        };
        let Some(from) = actions::parent(inv, id) else { continue };
        if filtered && !kind_wanted(prefs, asset_type) {
            // a kind left out of the sort stays where it is, unlisted
            continue;
        }
        if Some(id) == base || waiting_all.contains(&id) {
            // the sort folder or Pre-Trash, chosen by hand: they stay
            plan.kept += 1;
            plan.kept_list.push((name, Kept::Protected));
            continue;
        }
        if is_folder {
            let unloaded = unloaded_below(inv, id);
            if !unloaded.is_empty() {
                // moved whole once all it holds is loaded (and seen not worn)
                plan.unfetched.extend(unloaded);
                plan.kept += 1;
                plan.kept_list.push((name, Kept::Loading));
                continue;
            }
        }
        if movable(inv, id, protected, &holds_worn) && !prefs.excludes(inv, id) && !holds_closed(inv, id, &prefs.excluded) {
            let sorted = base.is_some_and(|b| from == b || actions::under(inv, from, b));
            let creator = if by_creator { creator_of(inv, id) } else { None };
            candidates.push((id, is_folder, from, name, asset_type, flags, created, sorted, creator));
        } else {
            plan.kept += 1;
            plan.kept_list.push((name, kept_reason(inv, id, prefs, protected, &holds_worn)));
        }
    }
    // Reading thousands of names against every phrase is the heavy part:
    // it runs on all cores, the inventory itself is only read above.
    use rayon::prelude::*;
    let read: Vec<_> = candidates
        .into_par_iter()
        .map(|(id, folder, from, name, asset_type, flags, created, sorted, creator)| {
            let rules = if sorted { &sorted_rules } else { &prefs.rules };
            let to = if forced.contains(&id) {
                Some(Dest::PreTrash)
            } else {
                target_by(&name, asset_type, flags, created, rules, prefs.stores, &prefs.store_hints, creator)
            };
            let unpacker = classify(&name, asset_type, flags).unpacker;
            (id, folder, from, name, to, unpacker)
        })
        .collect();
    let base_name = base_name_in(inv, prefs);
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
            restore: false,
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
    /// The next mutation, and the moves of the queue it carries (put back
    /// at the front if the batch is refused or never sent).
    Change(Box<Mutation>, Vec<Move>),
    /// A destination folder is loading: ask again next frame.
    Wait,
    /// A folder to move whole is loading its subfolders: ask again next
    /// frame (only that move gives up after a while).
    Loading,
    Done,
}

/// The next mutation of a sort: at most [`BATCH`] moves from the front of
/// `queue`, with the folders they need. A folder that exists (any case) is
/// reused; a missing one is created in the same mutation under a new id
/// that later operations name. Entries that can no longer move (worn,
/// deleted, moved away from its folder meanwhile) are dropped.
pub fn step(
    inv: &mut Inventory,
    base_name: &str,
    queue: &mut VecDeque<Move>,
    protected: &HashSet<Uuid>,
    worn: &HashSet<Uuid>,
    skipped: &mut usize,
) -> Step {
    let holds_worn = actions::worn_ancestors(inv, worn);
    let mut change = Mutation::default();
    // folders known in this mutation: (parent, lowercase name) → id
    let mut made: HashMap<(Uuid, String), Uuid> = HashMap::new();
    let mut fresh: HashSet<Uuid> = HashSet::new();
    let mut sent: Vec<Move> = Vec::new();
    let base_path = [base_name.to_owned()];
    // a sort folder name read back from disk (a resumed run) is checked
    // again: never a viewer folder, a system folder or Pre-Trash
    let base_ok = !base_refused(base_name);
    // the nearest existing folder above each folder made in this batch
    let mut anchor: HashMap<Uuid, Uuid> = HashMap::new();
    // what this batch moves and where it sends, for the moves after
    let mut sent_folders: Vec<Uuid> = Vec::new();
    let mut used: Vec<Uuid> = Vec::new();
    'moves: while sent.len() < BATCH {
        let Some(next) = queue.front() else { break };
        // a folder whose tree is still loading waits for it (Resume and
        // Undo read runs in folders not opened this session): skipped only
        // once it is loaded and still cannot move
        // (the cheap checks first: a worn, locked or protected folder is
        // never loaded only to be skipped)
        if next.folder
            && actions::parent(inv, next.id) == Some(next.from)
            && actions::relocatable(inv, next.id, protected)
            && !holds_worn.contains(&next.id)
        {
            let loading = unloaded_below(inv, next.id);
            if !loading.is_empty() {
                // a few at a time beside the inventory's own loading, never
                // a whole deep tree in one frame
                let reading = loading
                    .iter()
                    .filter(|id| inv.folders.get(id).is_some_and(|f| f.state == FetchState::Fetching))
                    .count();
                let ask: Vec<Uuid> = loading
                    .iter()
                    .copied()
                    .filter(|id| inv.folders.get(id).is_some_and(|f| f.state == FetchState::Unknown))
                    .take(LOADS_AT_ONCE.saturating_sub(reading))
                    .collect();
                for id in ask {
                    inv.request(id);
                }
                if change.operations.is_empty() {
                    return Step::Loading;
                }
                break 'moves;
            }
        }
        let still_here = actions::parent(inv, next.id) == Some(next.from)
            && inv.folders.contains_key(&next.id) == next.folder
            // a restore goes back to a folder, never along a path or to
            // Pre-Trash (a run file edited by hand cannot make it)
            && (!next.restore || matches!(next.to, Dest::Folder(_)))
            && if next.restore {
                restorable(inv, next.id, protected, &holds_worn)
            } else {
                movable(inv, next.id, protected, &holds_worn)
            };
        if !still_here {
            queue.pop_front();
            *skipped += 1;
            continue;
        }
        let path: Vec<String> = match &next.to {
            Dest::Folder(target) => {
                let allowed = if next.restore {
                    restore_target_allowed(inv, *target)
                } else {
                    target_allowed(inv, *target)
                };
                if !allowed {
                    queue.pop_front();
                    *skipped += 1;
                    continue;
                }
                Vec::new()
            }
            Dest::Path(_) if !base_ok => {
                queue.pop_front();
                *skipped += 1;
                continue;
            }
            Dest::Path(path) => base_path.iter().chain(path).cloned().collect(),
            // the Pre-Trash folder as it is named, in any language
            Dest::PreTrash => vec![
                pre_trash(inv)
                    .and_then(|p| inv.folders.get(&p))
                    .map_or_else(pre_trash_name, |f| f.info.name.clone()),
            ],
        };
        let to_pre_trash = matches!(next.to, Dest::PreTrash);
        let mut parent = match next.to {
            Dest::Folder(target) => target,
            Dest::Path(_) | Dest::PreTrash => inv.root,
        };
        // The folders this move would create: kept apart until the whole
        // path resolves, so a wait leaves no half-made path in `change`.
        let mut new_folders = Vec::new();
        // every existing folder the path goes through (the start included)
        let mut walked = vec![parent];
        for part in path {
            let key = (parent, part.to_lowercase());
            if let Some(id) = made.get(&key) {
                parent = *id;
                walked.push(anchor.get(id).copied().unwrap_or(*id));
                continue;
            }
            if !fresh.contains(&parent) {
                match inv.folders.get(&parent).map(|f| f.state) {
                    Some(FetchState::Fetched) => {}
                    Some(FetchState::Failed) | None => {
                        // the destination cannot be read: this entry stays
                        queue.pop_front();
                        *skipped += 1;
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
                    // an existing folder on the way is re-checked like a
                    // rule's destination: open, and not in Pre-Trash
                    // (unless the move goes to Pre-Trash itself)
                    let open = folder_open(inv, id) && (to_pre_trash || !in_pre_trash(inv, id));
                    if !open {
                        queue.pop_front();
                        *skipped += 1;
                        continue 'moves;
                    }
                    made.insert(key, id);
                    walked.push(id);
                    parent = id;
                    continue;
                }
            }
            let id = Uuid::new_v4();
            anchor.insert(id, anchor.get(&parent).copied().unwrap_or(parent));
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
        // One batch is applied all at once: a move whose way goes through a
        // folder this batch moves, or a folder holding a place this batch
        // sends to, waits for the next batch, where it is checked against
        // the inventory as it then is (never into Pre-Trash's tree or into
        // itself by the back door).
        if let Some(next) = queue.front() {
            let through_moved = walked
                .iter()
                .any(|w| sent_folders.iter().any(|s| *w == *s || actions::under(inv, *w, *s)));
            let holds_used = next.folder && used.iter().any(|u| *u == next.id || actions::under(inv, *u, next.id));
            if (through_moved || holds_used) && !change.operations.is_empty() {
                break 'moves;
            }
        }
        let Some(m) = queue.pop_front() else { break };
        // a folder sent to the root never takes a name the viewer or Auto-sort
        // reads there (#RLV, Pre-Trash, the sort folder…)
        if m.folder
            && parent == inv.root
            && inv.folders.get(&m.id).is_some_and(|f| {
                let name = f.info.name.trim();
                base_refused(name)
                    || name.eq_ignore_ascii_case(base_name)
                    || default_base_names().iter().any(|n| n.eq_ignore_ascii_case(name))
            })
        {
            *skipped += 1;
            continue;
        }
        // into itself, or into a folder made inside itself: every existing
        // folder on the way is checked, a new one has none of its own
        let into_itself = walked.iter().any(|w| *w == m.id || actions::under(inv, *w, m.id));
        if parent == m.id || into_itself || actions::under(inv, parent, m.id) {
            // into itself: dropped before any folder is made for it
            for op in &new_folders {
                if let Operation::Folder { id, .. } = op {
                    made.retain(|_, made_id| made_id != id);
                    fresh.remove(id);
                    change.refresh.retain(|r| r != id);
                }
            }
            *skipped += 1;
            continue;
        }
        // the new folders' parents are read back too, so a folder made by a
        // batch that fails later is still found by the next run; a source is
        // not (a folder of 100,000 items read again each batch)
        for op in &new_folders {
            if let Operation::Folder { parent: above, .. } = op {
                change.refresh.push(*above);
            }
        }
        change.operations.append(&mut new_folders);
        change.operations.push(Operation::Patch {
            id: m.id,
            folder: m.folder,
            body: llsd_map! { "parent_id" => parent },
        });
        change.refresh.push(parent);
        if m.folder {
            sent_folders.push(m.id);
        }
        used.extend(walked.iter().copied());
        sent.push(m);
    }
    if change.operations.is_empty() {
        return Step::Done;
    }
    // each move read back by itself, never a destination's whole contents
    change.confirm_each = true;
    change.refresh.sort_unstable();
    change.refresh.dedup();
    Step::Change(Box::new(change), sent)
}

#[path = "sorter_demo.rs"]
pub mod demo;

#[path = "sorter_share.rs"]
pub mod share;
#[path = "sorter_vocab.rs"]
pub mod vocab;

#[path = "sorter_journal.rs"]
pub mod journal;

#[cfg(test)]
#[path = "sorter_tests.rs"]
mod tests;
