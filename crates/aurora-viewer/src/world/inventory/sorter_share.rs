//! Auto-sort settings as plain text, to hand to someone else (pasted in a
//! notecard, a chat, a forum): the word rules, the store hints and the
//! options that mean the same in any inventory. A rule that points at one of
//! the user's own folders, and the excluded folders, are folder ids of this
//! inventory: they are never written (a comment line names such a rule).
//!
//! ```text
//! # Aurora Auto-sort v1
//! rule: any | denim jeans | Clothes/Denim
//! rule: all | old freebie | pre-trash | except-sorted
//! store: Birch Hollow = furniture
//! option: base = Sorted
//! option: stores = on
//! option: folders = on
//! option: kinds = objects, clothing, notecards
//! ```
//!
//! Reading is defensive: unknown or broken lines are skipped and counted,
//! the text and the number of rules are capped.

use super::{Category, Kind, Match, Preferences, Rule, StoreHint};
use uuid::Uuid;

pub const HEADER: &str = "# Aurora Auto-sort v1";
/// The longest text read, in bytes.
pub const MAX_TEXT: usize = 64 * 1024;
/// The most rules (and the most store hints) read.
pub const MAX_RULES: usize = 500;

impl Category {
    /// A stable English key, the same in every language.
    pub fn key(self) -> &'static str {
        match self {
            Category::Tattoos => "tattoos",
            Category::Skins => "skins",
            Category::Makeup => "makeup",
            Category::Hair => "hair",
            Category::Eyes => "eyes",
            Category::Nails => "nails",
            Category::Shoes => "shoes",
            Category::Stockings => "stockings",
            Category::Lingerie => "lingerie",
            Category::Swimwear => "swimwear",
            Category::Bodysuits => "bodysuits",
            Category::Dresses => "dresses",
            Category::Skirts => "skirts",
            Category::Pants => "pants",
            Category::Shorts => "shorts",
            Category::Jackets => "jackets",
            Category::Tops => "tops",
            Category::Sleepwear => "sleepwear",
            Category::Gloves => "gloves",
            Category::Jewelry => "jewelry",
            Category::Accessories => "accessories",
            Category::BodyAddons => "body-addons",
            Category::Heads => "heads",
            Category::Shapes => "shapes",
            Category::Animations => "animations",
            Category::Huds => "huds",
            Category::Furniture => "furniture",
            Category::Decor => "decor",
            Category::Vehicles => "vehicles",
            Category::Pets => "pets",
            Category::Outfits => "outfits",
            Category::Landmarks => "landmarks",
            Category::Notecards => "notecards",
            Category::Scripts => "scripts",
            Category::Sounds => "sounds",
            Category::Textures => "textures",
            Category::Gestures => "gestures",
        }
    }

    pub fn from_key(key: &str) -> Option<Category> {
        Category::ALL.into_iter().find(|c| c.key() == key)
    }
}

impl Kind {
    pub fn key(self) -> &'static str {
        match self {
            Kind::Objects => "objects",
            Kind::Clothing => "clothing",
            Kind::BodyParts => "body-parts",
            Kind::Notecards => "notecards",
            Kind::Landmarks => "landmarks",
            Kind::Textures => "textures",
            Kind::Scripts => "scripts",
            Kind::Sounds => "sounds",
            Kind::Animations => "animations",
            Kind::Gestures => "gestures",
            Kind::Materials => "materials",
            Kind::CallingCards => "calling-cards",
        }
    }

    pub fn from_key(key: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.key() == key)
    }
}

fn match_key(mode: Match) -> &'static str {
    match mode {
        Match::All => "all",
        Match::Any => "any",
        Match::Phrase => "phrase",
    }
}

/// `|` and line breaks would break a line: they become spaces.
fn clean(text: &str) -> String {
    text.replace(['|', '\n', '\r'], " ").trim().to_owned()
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

/// A rule another inventory can use: not one pointing at a folder of this
/// one.
pub fn shareable(rule: &Rule) -> bool {
    rule.pre_trash || rule.target.is_none()
}

/// What the user ticked in the Share window, by index of `prefs.rules` and
/// `prefs.store_hints`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    pub rules: Vec<bool>,
    pub hints: Vec<bool>,
    pub base: bool,
    pub stores: bool,
    pub folders: bool,
    pub kinds: bool,
}

impl Share {
    /// Everything shareable ticked: the user unticks their own exceptions.
    pub fn all(prefs: &Preferences) -> Self {
        Self {
            rules: prefs.rules.iter().map(shareable).collect(),
            hints: vec![true; prefs.store_hints.len()],
            base: !prefs.base.trim().is_empty(),
            stores: true,
            folders: true,
            kinds: true,
        }
    }
}

/// The ticked settings as text, ready for the clipboard. A rule to one of
/// the user's own folders is never written, ticked or not: a comment line
/// names it.
pub fn export(prefs: &Preferences, share: &Share) -> String {
    let mut out = vec![HEADER.to_owned()];
    for (index, rule) in prefs.rules.iter().enumerate() {
        let words = clean(&rule.words);
        if !shareable(rule) {
            // a folder of this inventory: it means nothing elsewhere, and its
            // words are the user's own business
            out.push("# a rule to one of my own folders is not shared".to_owned());
            continue;
        }
        if !share.rules.get(index).copied().unwrap_or(false) {
            continue;
        }
        let to = if rule.pre_trash {
            "pre-trash".to_owned()
        } else {
            clean(&rule.folder)
        };
        let mut flags = Vec::new();
        if rule.skip_sorted {
            flags.push("except-sorted");
        }
        if rule.inside_deliveries {
            flags.push("inside-deliveries");
        }
        if !rule.enabled {
            flags.push("off");
        }
        let mut line = format!("rule: {} | {words} | {to}", match_key(rule.mode));
        if !flags.is_empty() {
            line.push_str(" | ");
            line.push_str(&flags.join(", "));
        }
        out.push(line);
    }
    for (index, hint) in prefs.store_hints.iter().enumerate() {
        let store = clean(&hint.store).replace('=', " ");
        if !share.hints.get(index).copied().unwrap_or(false) {
            continue;
        }
        match hint.creator {
            // a creator's id is public (an item's Properties show it): it may
            // travel with the store it stands for
            Some(creator) => out.push(format!(
                "creator: {creator} = {} | {} | {}",
                store.replace('|', " "),
                hint.category.key(),
                clean(&hint.creator_name).replace('|', " ")
            )),
            None if !store.is_empty() => out.push(format!("store: {store} = {}", hint.category.key())),
            None => {}
        }
    }
    if share.base && !prefs.base.trim().is_empty() {
        out.push(format!("option: base = {}", clean(&prefs.base)));
    }
    if share.stores {
        out.push(format!("option: stores = {}", on_off(prefs.stores)));
    }
    if share.folders {
        out.push(format!("option: folders = {}", on_off(prefs.folders)));
    }
    // no type at all is no list a text can carry (an import refuses it)
    if share.kinds && !prefs.kinds.is_empty() {
        out.push(format!(
            "option: kinds = {}",
            prefs.kinds.iter().map(|k| k.key()).collect::<Vec<_>>().join(", ")
        ));
    }
    out.join("\n") + "\n"
}

/// What a pasted text holds.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Imported {
    pub rules: Vec<Rule>,
    pub hints: Vec<StoreHint>,
    pub base: Option<String>,
    pub stores: Option<bool>,
    pub folders: Option<bool>,
    pub kinds: Option<Vec<Kind>>,
    /// Lines that could not be read (comments and blank lines are not
    /// counted).
    pub skipped: usize,
    /// The text began with the header.
    pub recognised: bool,
}

impl Imported {
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
            && self.hints.is_empty()
            && self.base.is_none()
            && self.stores.is_none()
            && self.folders.is_none()
            && self.kinds.is_none()
    }

    /// Which rules and store hints the user already has: shown "already in
    /// yours" and unticked in the Import window.
    pub fn duplicates(&self, prefs: &Preferences) -> (Vec<bool>, Vec<bool>) {
        let rules = self
            .rules
            .iter()
            .map(|r| prefs.rules.iter().any(|mine| same_rule(mine, r)))
            .collect();
        let hints = self
            .hints
            .iter()
            .map(|h| prefs.store_hints.iter().any(|mine| same_hint(mine, h)))
            .collect();
        (rules, hints)
    }

    /// The ticked items into `prefs`: added after the user's own rules and
    /// hints (a store already hinted keeps the user's), or replacing them.
    /// The options are set when `options` is ticked. Excluded folders are
    /// never shared, so they stay either way.
    pub fn apply(&self, prefs: &mut Preferences, pick: &Pick, replace: bool) {
        if replace {
            // what the file holds too stays (it is shown "already in
            // yours", never ticked); so do the rules to the user's own
            // folders, which no file can carry
            prefs
                .rules
                .retain(|mine| mine.target.is_some() || self.rules.iter().any(|r| same_rule(mine, r)));
            prefs.store_hints.retain(|mine| self.hints.iter().any(|h| same_hint(mine, h)));
        }
        for (index, rule) in self.rules.iter().enumerate() {
            let ticked = pick.rules.get(index).copied().unwrap_or(false);
            if ticked && !prefs.rules.iter().any(|mine| same_rule(mine, rule)) && prefs.rules.len() < MAX_RULES {
                prefs.rules.push(rule.clone());
            }
        }
        for (index, hint) in self.hints.iter().enumerate() {
            let ticked = pick.hints.get(index).copied().unwrap_or(false);
            let known = prefs.store_hints.iter().any(|h| same_hint(h, hint));
            if ticked && !known && prefs.store_hints.len() < MAX_RULES {
                prefs.store_hints.push(hint.clone());
            }
        }
        if pick.base
            && let Some(base) = &self.base
            && prefs.base != *base
        {
            prefs.base.clone_from(base);
            prefs.base_id = None;
        }
        if pick.stores
            && let Some(stores) = self.stores
        {
            prefs.stores = stores;
        }
        if pick.folders
            && let Some(folders) = self.folders
        {
            prefs.folders = folders;
        }
        if pick.kinds
            && let Some(kinds) = &self.kinds
        {
            prefs.kinds.clone_from(kinds);
        }
    }

    /// The options this text would change, as (which, now, then): the
    /// Import window lists each one old → new, highlighted, apart from the
    /// rules. An option the text sets to what the user already has is not
    /// a change.
    pub fn changes(&self, prefs: &Preferences) -> Vec<(OptionKind, String, String)> {
        let kinds = |k: &[Kind]| k.iter().map(|k| k.key()).collect::<Vec<_>>().join(", ");
        let mut out = Vec::new();
        // an empty name is the default one: compared and shown as it is used
        let mine = prefs.base_name();
        if let Some(base) = &self.base
            && !base.trim().is_empty()
            // a name the sort would refuse (a viewer or system folder's) is
            // no change: it would never be used
            && !super::base_refused(&super::inventory_name(base.trim(), ""))
            && base.trim() != mine
        {
            out.push((OptionKind::Base, mine, base.trim().to_owned()));
        }
        if let Some(stores) = self.stores
            && stores != prefs.stores
        {
            out.push((OptionKind::Stores, on_off(prefs.stores).into(), on_off(stores).into()));
        }
        if let Some(folders) = self.folders
            && folders != prefs.folders
        {
            out.push((OptionKind::Folders, on_off(prefs.folders).into(), on_off(folders).into()));
        }
        if let Some(new) = &self.kinds
            && *new != prefs.kinds
        {
            out.push((OptionKind::Kinds, kinds(&prefs.kinds), kinds(new)));
        }
        out
    }

    /// How many of the user's defaults the text changes: each option that
    /// differs, and each Pre-Trash rule the user does not have yet (it sends
    /// things towards the Trash).
    pub fn defaults_changed(&self, prefs: &Preferences) -> usize {
        let (known, _) = self.duplicates(prefs);
        let new_pre_trash = self.rules.iter().zip(known).filter(|(r, known)| r.pre_trash && !known).count();
        self.changes(prefs).len() + new_pre_trash
    }
}

/// An option a shared text may set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKind {
    Base,
    Stores,
    Folders,
    Kinds,
}

/// What the user ticked in the Import window: each rule, each hint and each
/// option apart.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pick {
    pub rules: Vec<bool>,
    pub hints: Vec<bool>,
    pub base: bool,
    pub stores: bool,
    pub folders: bool,
    pub kinds: bool,
}

impl Pick {
    /// What a file can safely bring is ticked, except what the user already
    /// has. Left for the user to tick: every Pre-Trash rule (it sends things
    /// towards the Trash) and a new sort folder name (it renames where
    /// everything goes).
    pub fn new(found: &Imported, prefs: &Preferences) -> Self {
        let (rules, hints) = found.duplicates(prefs);
        Self {
            rules: rules
                .into_iter()
                .zip(&found.rules)
                .map(|(dup, rule)| !dup && !rule.pre_trash)
                .collect(),
            hints: hints.into_iter().map(|dup| !dup).collect(),
            base: false,
            stores: true,
            folders: true,
            kinds: true,
        }
    }
}

/// The same hint: the same creator, or for store-name hints the same store.
fn same_hint(a: &StoreHint, b: &StoreHint) -> bool {
    match (a.creator, b.creator) {
        (Some(x), Some(y)) => x == y,
        (None, None) => a.store.trim().eq_ignore_ascii_case(b.store.trim()),
        _ => false,
    }
}

/// The same rule, whatever the case and spacing of its words and path.
fn same_rule(a: &Rule, b: &Rule) -> bool {
    let norm = |s: &str| super::words(s).join(" ");
    let same_words = a.mode == b.mode
        && norm(&a.words) == norm(&b.words)
        && a.skip_sorted == b.skip_sorted
        && a.inside_deliveries == b.inside_deliveries;
    if a.pre_trash || b.pre_trash {
        // a Pre-Trash rule has no folder: its words, where it looks and
        // whether it is on make it
        return same_words && a.pre_trash == b.pre_trash;
    }
    same_words && a.target == b.target && super::split_path(&a.folder) == super::split_path(&b.folder)
}

fn read_bool(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "on" | "yes" | "true" | "1" => Some(true),
        "off" | "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

fn read_rule(value: &str) -> Option<Rule> {
    let parts: Vec<&str> = value.split('|').map(str::trim).collect();
    let (mode, words, to) = (parts.first()?, parts.get(1)?, parts.get(2)?);
    let mode = match mode.to_lowercase().as_str() {
        "all" => Match::All,
        "any" => Match::Any,
        "phrase" => Match::Phrase,
        _ => return None,
    };
    let flags: Vec<String> = parts
        .get(3)
        .map(|f| f.split(',').map(|f| f.trim().to_lowercase()).collect())
        .unwrap_or_default();
    let pre_trash = to.eq_ignore_ascii_case("pre-trash");
    let rule = Rule {
        enabled: !flags.iter().any(|f| f == "off"),
        words: words.chars().take(120).collect(),
        mode,
        folder: if pre_trash { String::new() } else { to.chars().take(200).collect() },
        pre_trash,
        skip_sorted: flags.iter().any(|f| f == "except-sorted"),
        inside_deliveries: flags.iter().any(|f| f == "inside-deliveries"),
        ..Default::default()
    };
    // a rule with no words or nowhere to go is a broken line
    let usable = Rule {
        enabled: true,
        ..rule.clone()
    }
    .usable();
    usable.then_some(rule)
}

fn read_hint(value: &str) -> Option<StoreHint> {
    let (store, category) = value.rsplit_once('=')?;
    let store: String = store.trim().chars().take(64).collect();
    let category = Category::from_key(category.trim().to_lowercase().as_str())?;
    (!store.is_empty()).then_some(StoreHint {
        store,
        category,
        ..Default::default()
    })
}

/// `creator: <id> = <store> | <category> | <name>`.
fn read_creator(value: &str) -> Option<StoreHint> {
    let (creator, rest) = value.split_once('=')?;
    let creator = Uuid::parse_str(creator.trim()).ok().filter(|c| !c.is_nil())?;
    let parts: Vec<&str> = rest.split('|').map(str::trim).collect();
    let category = Category::from_key(parts.get(1)?.to_lowercase().as_str())?;
    Some(StoreHint {
        store: parts.first()?.chars().take(64).collect(),
        category,
        creator: Some(creator),
        creator_name: parts.get(2).map(|n| n.chars().take(64).collect()).unwrap_or_default(),
    })
}

/// Read a pasted text. Never fails: what cannot be read is counted in
/// `skipped`.
pub fn import(text: &str) -> Imported {
    let mut found = Imported::default();
    let text = if text.len() > MAX_TEXT {
        // cut on a character boundary
        let mut end = MAX_TEXT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        &text[..end]
    } else {
        text
    };
    for line in text.lines().map(str::trim) {
        if line.eq_ignore_ascii_case(HEADER) {
            found.recognised = true;
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            found.skipped += 1;
            continue;
        };
        let read = match key.trim().to_lowercase().as_str() {
            "rule" if found.rules.len() < MAX_RULES => read_rule(value).map(|r| found.rules.push(r)),
            "store" if found.hints.len() < MAX_RULES => read_hint(value).map(|h| found.hints.push(h)),
            "creator" if found.hints.len() < MAX_RULES => read_creator(value).map(|h| found.hints.push(h)),
            "option" => value.split_once('=').and_then(|(name, value)| {
                let value = value.trim();
                match name.trim().to_lowercase().as_str() {
                    "base" => {
                        found.base = Some(value.chars().take(63).collect());
                        Some(())
                    }
                    "stores" => read_bool(value).map(|v| found.stores = Some(v)),
                    "folders" => read_bool(value).map(|v| found.folders = Some(v)),
                    "kinds" => {
                        let kinds: Option<Vec<Kind>> = value
                            .split(',')
                            .map(str::trim)
                            .filter(|k| !k.is_empty())
                            .map(|k| Kind::from_key(&k.to_lowercase()))
                            .collect();
                        // an empty list would sort nothing at all: not read
                        kinds.filter(|kinds| !kinds.is_empty()).map(|mut kinds| {
                            kinds.sort();
                            kinds.dedup();
                            found.kinds = Some(kinds);
                        })
                    }
                    _ => None,
                }
            }),
            _ => None,
        };
        if read.is_none() {
            found.skipped += 1;
        }
    }
    found
}
