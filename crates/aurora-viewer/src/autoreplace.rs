//! Auto-replace: keywords replaced by a word or a phrase as they are
//! typed. Port of LLAutoReplace and LLAutoReplaceSettings
//! (indra/newview/llautoreplace.cpp, originally LGPL 2.1) and of the
//! AutoReplace setting (off by default):
//! - the lists are tried in their order, the first one holding the word
//!   wins (replaceWord); a keyword matches exactly, case included;
//! - the word is replaced when a space is typed right after it
//!   (autoreplaceCallback); its characters are letters, digits, `_` and
//!   `( ) . , -` (FIRE-36304);
//! - the lists are kept in `<config>/autoreplace.xml`, the Legacy LLSD
//!   format, the bundled ones (app_settings/autoreplace.xml: Abbreviations,
//!   Spelling Corrections) until the user changes them; a list is
//!   imported and exported alone in that same format, so the Legacy files
//!   load as they are.

use aurora_llsd::Llsd;
use parking_lot::RwLock;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

/// The Legacy Viewer's default lists.
static BUNDLED: &str = include_str!("../assets/autoreplace.xml");

/// One list of replacements.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct List {
    pub name: String,
    /// keyword → replacement (a std::map in the Legacy Viewer: sorted).
    pub replacements: BTreeMap<String, String>,
}

impl List {
    /// LLAutoReplaceSettings::listIsValid: a name and a map of strings.
    pub fn from_llsd(v: &Llsd) -> Option<List> {
        let name = match v.get("name") {
            Llsd::String(name) if !name.trim().is_empty() => name.clone(),
            _ => return None,
        };
        let map = v.get("replacements").as_map()?;
        let mut replacements = BTreeMap::new();
        for (keyword, replacement) in map {
            let Llsd::String(replacement) = replacement else {
                return None;
            };
            if !keyword.is_empty() {
                replacements.insert(keyword.clone(), replacement.clone());
            }
        }
        Some(List { name, replacements })
    }

    pub fn to_llsd(&self) -> Llsd {
        let mut replacements = Llsd::new_map();
        for (keyword, replacement) in &self.replacements {
            replacements.insert(keyword.clone(), replacement.clone());
        }
        let mut list = Llsd::new_map();
        list.insert("name", self.name.clone());
        list.insert("replacements", replacements);
        list
    }
}

/// The lists of an LLSD file (an array of lists); invalid ones are left out.
pub fn parse_lists(bytes: &[u8]) -> Result<Vec<List>, String> {
    let root = aurora_llsd::from_xml(bytes).map_err(|e| e.to_string())?;
    if !root.is_array() {
        return Err("not a list of replacement lists".into());
    }
    Ok(root.as_array().iter().filter_map(List::from_llsd).collect())
}

pub fn lists_to_xml(lists: &[List]) -> String {
    let mut root = Llsd::new_array();
    for list in lists {
        root.push(list.to_llsd());
    }
    aurora_llsd::to_xml_string(&root)
}

/// AutoReplace (Preferences › Chat › Spelling), installed by the app.
static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// The bundled lists, parsed once: some 4,000 entries, too many to parse
/// again each time they are compared or restored (the Spelling page did it
/// every frame, #1363).
static DEFAULTS: LazyLock<Arc<Vec<List>>> = LazyLock::new(|| Arc::new(bundled()));

/// The lists in use (the bundled ones until the user's file is read).
static LISTS: LazyLock<RwLock<Arc<Vec<List>>>> = LazyLock::new(|| RwLock::new(DEFAULTS.clone()));

fn bundled() -> Vec<List> {
    parse_lists(BUNDLED.as_bytes())
        .map_err(|e| log::error!("autoreplace: the bundled lists: {e}"))
        .unwrap_or_default()
}

fn user_file() -> PathBuf {
    crate::settings::config_dir().join("autoreplace.xml")
}

/// Reads the user's lists on a thread (LLAutoReplace::loadFromSettings);
/// without a file, the bundled ones stay.
pub fn load() {
    let spawned = std::thread::Builder::new().name("aurora-autoreplace".into()).spawn(|| {
        let path = user_file();
        match std::fs::read(&path) {
            Ok(bytes) => match parse_lists(&bytes) {
                Ok(lists) => *LISTS.write() = shared(lists),
                Err(e) => log::warn!("autoreplace: {}: {e}", path.display()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("autoreplace: {}: {e}", path.display()),
        }
    });
    if let Err(e) = spawned {
        log::warn!("autoreplace: no thread: {e}");
    }
}

pub fn lists() -> Arc<Vec<List>> {
    LISTS.read().clone()
}

/// The lists changed in the preferences, saved on a thread
/// (LLAutoReplace::saveToUserSettings).
pub fn set_lists(lists: Vec<List>) {
    let xml = lists_to_xml(&lists);
    *LISTS.write() = shared(lists);
    let spawned = std::thread::Builder::new().name("aurora-autoreplace-save".into()).spawn(move || {
        let path = user_file();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = crate::atomic_file::write(&path, xml.as_bytes()) {
            log::warn!("autoreplace: {}: {e}", path.display());
        }
    });
    if let Err(e) = spawned {
        log::warn!("autoreplace: no thread: {e}");
    }
}

/// The bundled lists again.
pub fn defaults() -> Arc<Vec<List>> {
    DEFAULTS.clone()
}

/// Whether `lists` are the bundled ones: the same `Arc` most of the time
/// ([`shared`]), so the page checks it each frame for nothing.
pub fn is_default(lists: &Arc<Vec<List>>) -> bool {
    Arc::ptr_eq(lists, &DEFAULTS) || **lists == **DEFAULTS
}

/// Lists equal to the bundled ones share their `Arc`.
fn shared(lists: Vec<List>) -> Arc<Vec<List>> {
    if lists == **DEFAULTS { DEFAULTS.clone() } else { Arc::new(lists) }
}

/// LLAutoReplaceSettings::replaceWord: the replacement of the first list
/// holding `word`.
pub fn replacement<'a>(lists: &'a [List], word: &str) -> Option<&'a str> {
    lists.iter().find_map(|list| list.replacements.get(word)).map(String::as_str)
}

/// isAutoReplaceKeywordChar (FIRE-36304).
fn keyword_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '(' | ')' | '.' | ',' | '-')
}

/// LLAutoReplaceSettings::addEntryToList: a keyword is one word of
/// keyword characters.
pub fn valid_keyword(keyword: &str) -> bool {
    !keyword.is_empty() && keyword.chars().all(keyword_char)
}

/// LLAutoReplace::autoreplaceCallback, after a space typed at the
/// character cursor `cursor`: the text with the word before it replaced,
/// and the new cursor.
pub fn after_space(lists: &[List], text: &str, cursor: usize) -> Option<(String, usize)> {
    let chars: Vec<char> = text.chars().collect();
    if cursor < 2 || cursor > chars.len() || chars[cursor - 1] != ' ' {
        return None;
    }
    let end = cursor - 1;
    let start = (0..end).rev().take_while(|&i| keyword_char(chars[i])).last()?;
    let word: String = chars[start..end].iter().collect();
    let with = replacement(lists, &word)?;
    let mut out: String = chars[..start].iter().collect();
    out.push_str(with);
    out.extend(&chars[end..]);
    Some((out, cursor - (end - start) + with.chars().count()))
}

/// An exported list's file name (LLFloaterAutoReplaceSettings::onExportList:
/// the list name with its spaces as `_`).
pub fn export_name(list: &str) -> String {
    let clean: String = list
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
        .collect();
    format!("{clean}.xml")
}

/// One list of a Legacy export file (a single list, LLSD).
pub fn parse_exported(bytes: &[u8]) -> Result<List, String> {
    let root = aurora_llsd::from_xml(bytes).map_err(|e| e.to_string())?;
    List::from_llsd(&root).ok_or_else(|| "not a replacement list".into())
}

pub fn exported_xml(list: &List) -> String {
    aurora_llsd::to_xml_string(&list.to_llsd())
}

/// A name not yet taken by `lists` (LLAutoReplaceSettings::listNameIsUnique;
/// the Legacy Viewer asked again, here " 2", " 3"… is added).
pub fn unique_name(lists: &[List], wanted: &str) -> String {
    let wanted = wanted.trim();
    if !lists.iter().any(|l| l.name == wanted) {
        return wanted.to_owned();
    }
    (2..)
        .map(|n| format!("{wanted} {n}"))
        .find(|name| !lists.iter().any(|l| &l.name == name))
        .unwrap_or_else(|| wanted.to_owned())
}

#[cfg(test)]
mod tests;
