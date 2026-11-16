//! Auto-sort's vocabulary: the words creators use in item names for bodies,
//! categories, skin tones, unpackers and variants, and the ways they tag
//! their store. It lives in `assets/auto-sort/vocabulary.json` so that a
//! word can be added without reading the sorter's code; the golden corpus
//! (`assets/auto-sort/corpus.json`, its test in `sorter_vocab_tests.rs`)
//! shows what a change does to real-looking names.
//!
//! Development builds (any profile but `release`) read another file when
//! `AURORA_SORTER_VOCAB=<path>` is set, to try a change without
//! rebuilding; a file that does not read falls back to the built-in one,
//! with a warning in the log.

use super::{Category, words};
use serde::Deserialize;
use std::sync::LazyLock;

const BUILT_IN: &str = include_str!("../../../assets/auto-sort/vocabulary.json");

/// A body, its words, and the bodies a match rules out (eBODY Reborn is
/// eBODY's Reborn body, not its older Classic one). Weak words only count
/// when no strong word names a body: "Lara" or "Juicy" are also ordinary
/// words in item names.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BodyWords {
    pub body: String,
    pub strong: Vec<String>,
    #[serde(default)]
    pub weak: Vec<String>,
    #[serde(default)]
    pub hides: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CategoryEntry {
    category: String,
    words: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variants {
    /// A Squish or Squeeze version (`Reborn bra black sq`).
    pub squish: Vec<String>,
    /// Made for Reborn's Waifus mod (`Reborn Waifus - Bra`).
    pub waifus: Vec<String>,
}

/// How a name starts with its store: enclosed (`[Store]`, `*Store*`) or
/// before a separator (`Store - Dress`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreTags {
    pub enclosed: Vec<(char, char)>,
    pub separators: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    #[serde(default)]
    #[allow(dead_code)]
    about: String,
    bodies: Vec<BodyWords>,
    categories: Vec<CategoryEntry>,
    skin_tones: Vec<String>,
    unpackers: Vec<String>,
    variants: Variants,
    store_tag_patterns: StoreTags,
    #[serde(default)]
    store_stopwords: Vec<String>,
}

/// The vocabulary, read and checked.
#[derive(Debug, Clone, Default)]
pub struct Vocabulary {
    pub bodies: Vec<BodyWords>,
    /// In the order they are tried: the first match wins, so a "Lingerie
    /// Set" is lingerie. Sets come last.
    pub categories: Vec<(Category, Vec<String>)>,
    /// Skin tones creators name their body skins with (`(Porcelain)`).
    pub skin_tones: Vec<String>,
    /// A box to wear or rez that gives the real items.
    pub unpackers: Vec<String>,
    pub variants: Variants,
    pub store_tags: StoreTags,
    /// Words of a tag that names a status, not a store (`(Mesh)`, `[DEMO]`,
    /// `*NEW*`): a tag made only of them is no store.
    pub store_stopwords: Vec<String>,
}

impl Vocabulary {
    /// Read a vocabulary file. Refused, with the reason: an unknown section
    /// or category, an empty body, a body hiding a body that does not exist,
    /// and a word written so that no name could ever match it (capitals,
    /// punctuation, double spaces: names are read as lowercase words).
    pub fn parse(text: &str) -> Result<Self, String> {
        let raw: Raw = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let mut problems = Vec::new();
        for body in &raw.bodies {
            check(&mut problems, &format!("bodies/{}", body.body), &body.strong);
            check(&mut problems, &format!("bodies/{}", body.body), &body.weak);
        }
        let mut categories = Vec::with_capacity(raw.categories.len());
        for entry in raw.categories {
            match Category::from_key(&entry.category) {
                Some(category) => {
                    check(&mut problems, &format!("categories/{}", entry.category), &entry.words);
                    categories.push((category, entry.words));
                }
                None => problems.push(format!("categories: unknown category \"{}\"", entry.category)),
            }
        }
        check(&mut problems, "skin_tones", &raw.skin_tones);
        check(&mut problems, "unpackers", &raw.unpackers);
        check(&mut problems, "variants/squish", &raw.variants.squish);
        check(&mut problems, "variants/waifus", &raw.variants.waifus);
        check(&mut problems, "store_stopwords", &raw.store_stopwords);
        // a body's name is a folder name as the inventory keeps it: anything
        // else would be renamed by the server and made again on each run
        for body in &raw.bodies {
            if super::inventory_name(&body.body, "") != body.body {
                problems.push(format!("bodies: \"{}\" is no inventory folder name", body.body));
            }
        }
        for body in &raw.bodies {
            if body.body.trim().is_empty() {
                problems.push("bodies: a body without a name".to_owned());
            }
            for hidden in &body.hides {
                if !raw.bodies.iter().any(|b| b.body == *hidden) {
                    problems.push(format!("bodies/{}: hides \"{hidden}\", which is no body", body.body));
                }
            }
        }
        if raw.store_tag_patterns.separators.iter().any(|s| s.trim().is_empty()) {
            problems.push("store_tag_patterns: an empty separator".to_owned());
        }
        if !problems.is_empty() {
            return Err(problems.join("; "));
        }
        Ok(Self {
            bodies: raw.bodies,
            categories,
            skin_tones: raw.skin_tones,
            unpackers: raw.unpackers,
            variants: raw.variants,
            store_tags: raw.store_tag_patterns,
            store_stopwords: raw.store_stopwords,
        })
    }

    /// The vocabulary built into the viewer.
    pub fn built_in() -> Self {
        // the built-in file is checked by the vocabulary's test: a broken
        // one never ships
        Self::parse(BUILT_IN).expect("assets/auto-sort/vocabulary.json reads (tested)")
    }
}

/// Each word as `sorter::words` reads names: a word written otherwise
/// (capitals, punctuation, double spaces) could never match.
fn check(problems: &mut Vec<String>, section: &str, list: &[String]) {
    for word in list {
        let read = words(word).join(" ");
        if read != *word || word.is_empty() {
            problems.push(format!("{section}: \"{word}\" is read as \"{read}\""));
        }
    }
}

/// The vocabulary in use: the built-in one, or in a development build the
/// file `AURORA_SORTER_VOCAB` names.
pub static VOCABULARY: LazyLock<Vocabulary> = LazyLock::new(|| {
    if let Some(path) = override_path() {
        match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| Vocabulary::parse(&text))
        {
            Ok(vocabulary) => {
                log::info!("Auto-sort: vocabulary read from {}", path.display());
                return vocabulary;
            }
            Err(e) => log::warn!("Auto-sort: {} not used, the built-in vocabulary is: {e}", path.display()),
        }
    }
    Vocabulary::built_in()
});

/// `AURORA_SORTER_VOCAB`, in development builds only: a published build
/// always sorts with the vocabulary it was tested with.
fn override_path() -> Option<std::path::PathBuf> {
    if env!("AURORA_BUILD_PROFILE") == "release" {
        return None;
    }
    std::env::var_os("AURORA_SORTER_VOCAB")
        .filter(|p| !p.is_empty())
        .map(std::path::PathBuf::from)
}

const CORPUS: &str = include_str!("../../../assets/auto-sort/corpus.json");

#[derive(Debug, Deserialize)]
struct CorpusFile {
    #[serde(default)]
    #[allow(dead_code)]
    about: String,
    cases: Vec<CorpusCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusCase {
    name: String,
    #[serde(rename = "type")]
    asset_type: i32,
    expect: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    why: String,
}

/// One name of the golden corpus: where it should go, where it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusRow {
    pub name: String,
    pub asset_type: i32,
    /// The expected path in this language's folder names; None: it stays.
    pub expected: Option<String>,
    pub got: Option<String>,
}

impl CorpusRow {
    pub fn matches(&self) -> bool {
        self.expected == self.got
    }
}

/// `{tops}/Store` with this language's folder names.
fn expand(path: &str) -> String {
    let mut out = path.replace("{stores}", &super::stores_folder());
    for category in Category::ALL {
        out = out.replace(&format!("{{{}}}", category.key()), &super::category_folder(category));
    }
    out
}

/// The corpus sorted with the vocabulary in use: a folder per store, no
/// rules, no hints. For the test and the `sorter-vocab` demo.
pub fn corpus() -> Vec<CorpusRow> {
    let file: CorpusFile = match serde_json::from_str(CORPUS) {
        Ok(file) => file,
        Err(e) => {
            log::warn!("Auto-sort: the corpus does not read: {e}");
            return Vec::new();
        }
    };
    file.cases
        .into_iter()
        .map(|case| {
            let got = match super::target(&case.name, case.asset_type, 0, None, &[], true) {
                Some(super::Dest::Path(path)) => Some(path.join("/")),
                Some(other) => Some(format!("{other:?}")),
                None => None,
            };
            CorpusRow {
                expected: case.expect.as_deref().map(expand),
                name: case.name,
                asset_type: case.asset_type,
                got,
            }
        })
        .collect()
}

/// The rows that differ, as a table a person reads: name, expected, got.
#[cfg(test)]
pub fn corpus_diff(rows: &[CorpusRow]) -> String {
    let shown = |p: &Option<String>| p.clone().unwrap_or_else(|| "(stays)".to_owned());
    rows.iter()
        .filter(|r| !r.matches())
        .map(|r| {
            format!(
                "{:?} (type {})\n    expected: {}\n    got:      {}",
                r.name,
                r.asset_type,
                shown(&r.expected),
                shown(&r.got)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
#[path = "sorter_vocab_tests.rs"]
mod tests;
