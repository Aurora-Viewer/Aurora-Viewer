//! The dictionaries: the ones bundled in the exe (the Legacy Viewer's
//! list, app_settings/dictionaries/dictionaries.xml) and the ones the user
//! imported, with the words they added or ignore. Port of the file side of
//! LLSpellChecker (indra/llui/llspellcheck.cpp, originally LGPL 2.1):
//! refreshDictionaryMap, initHunspell, addToDictFile, removeDictionary.
//!
//! The user's files are in `<config>/dictionaries`, with the Legacy names:
//! `<name>.aff` / `<name>.dic` for an imported dictionary (they win over a
//! bundled one of the same name, as the Legacy user path did), and
//! `user_custom.dic` / `user_ignore.dic`, a word per line after the count of
//! words. The list of the imported dictionaries is `user_dictionaries.json`
//! (the Legacy user_dictionaries.xml).

use serde::{Deserialize, Serialize};
use std::io::Read as _;
use std::path::{Path, PathBuf};

/// The name of the Second Life glossary, an extra dictionary (no affix
/// file) of the grid's words.
pub const GLOSSARY: &str = "sl";

const CUSTOM_FILE: &str = "user_custom.dic";
const IGNORE_FILE: &str = "user_ignore.dic";
const IMPORTED_FILE: &str = "user_dictionaries.json";

/// One dictionary of the list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Info {
    /// The file name without extension ("en_us").
    pub name: String,
    /// The name shown ("English (United States)").
    pub language: String,
    /// It has an affix file, so it can be the main dictionary ("is_primary").
    pub primary: bool,
    /// Imported by the user, so it can be removed ("user_installed").
    #[serde(skip)]
    pub imported: bool,
}

macro_rules! bundled {
    ($name:literal) => {
        include_bytes!(concat!(env!("OUT_DIR"), "/dictionaries/", $name, ".gz"))
    };
}

/// name, language, affix file, word list (gzipped by build.rs).
type Bundled = (&'static str, &'static str, Option<&'static [u8]>, &'static [u8]);

/// The Legacy Viewer's dictionaries, in its order; the languages are their
/// own names, as there.
const BUNDLED: [Bundled; 11] = [
    ("en_au", "English (Australia)", Some(bundled!("en_au.aff")), bundled!("en_au.dic")),
    (
        "en_gb",
        "English (United Kingdom)",
        Some(bundled!("en_gb.aff")),
        bundled!("en_gb.dic"),
    ),
    (
        "en_us",
        "English (United States)",
        Some(bundled!("en_us.aff")),
        bundled!("en_us.dic"),
    ),
    ("da_dk", "Dansk", Some(bundled!("da_dk.aff")), bundled!("da_dk.dic")),
    ("de_de", "Deutsch", Some(bundled!("de_de.aff")), bundled!("de_de.dic")),
    ("es_es", "Español (España)", Some(bundled!("es_es.aff")), bundled!("es_es.dic")),
    ("fr_fr", "Français", Some(bundled!("fr_fr.aff")), bundled!("fr_fr.dic")),
    ("pl_pl", "Polski", Some(bundled!("pl_pl.aff")), bundled!("pl_pl.dic")),
    ("pt_br", "Português (Brasil)", Some(bundled!("pt_br.aff")), bundled!("pt_br.dic")),
    ("ru_ru", "Русский", Some(bundled!("ru_ru.aff")), bundled!("ru_ru.dic")),
    (GLOSSARY, "", None, bundled!("sl.dic")),
];

/// The folder of the user's dictionaries and word lists.
pub fn user_dir() -> PathBuf {
    crate::settings::config_dir().join("dictionaries")
}

/// The bundled dictionaries then the imported ones (an imported one with a
/// bundled name replaces its files, not its entry).
pub fn catalogue(dir: &Path) -> Vec<Info> {
    let mut list: Vec<Info> = BUNDLED
        .iter()
        .map(|(name, language, aff, _)| Info {
            name: (*name).to_owned(),
            language: if *name == GLOSSARY {
                aurora_i18n::tr!("spelling-glossary").to_owned()
            } else {
                (*language).to_owned()
            },
            primary: aff.is_some(),
            imported: false,
        })
        .collect();
    for mut info in imported(dir) {
        info.imported = true;
        match list.iter_mut().find(|known| known.name == info.name) {
            Some(known) => known.primary |= info.primary,
            None => list.push(info),
        }
    }
    list
}

fn imported(dir: &Path) -> Vec<Info> {
    let Ok(bytes) = std::fs::read(dir.join(IMPORTED_FILE)) else {
        return Vec::new();
    };
    serde_json::from_slice::<Vec<Info>>(&bytes)
        .map_err(|e| log::warn!("spelling: {IMPORTED_FILE}: {e}"))
        .unwrap_or_default()
        .into_iter()
        // only plain file names: the entry names files of this folder
        .filter(|info| valid_name(&info.name) && dir.join(format!("{}.dic", info.name)).is_file())
        .collect()
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !name.starts_with('.')
}

fn inflate(gz: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    if let Err(e) = flate2::read::GzDecoder::new(gz).read_to_end(&mut out) {
        // built by build.rs from the repository: only a broken build gets here
        log::error!("spelling: a bundled dictionary does not inflate: {e}");
        out.clear();
    }
    out
}

/// The text of a dictionary file: UTF-8 (a BOM dropped), else the
/// encoding its affix file names (`SET ISO8859-1`…), as Hunspell reads it.
pub fn decode(bytes: &[u8], aff: Option<&[u8]>) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_owned();
    }
    let label = aff.unwrap_or(bytes).split(|&b| b == b'\n').find_map(|line| {
        let line = std::str::from_utf8(line).ok()?.trim();
        line.strip_prefix("SET")
            .filter(|rest| rest.starts_with(char::is_whitespace))
            .map(str::trim)
    });
    // Hunspell's names (ISO8859-1, microsoft-cp1251…) are WHATWG labels once
    // the dash is back ("iso-8859-1")
    let encoding = label
        .and_then(|label| {
            let lower = label.to_ascii_lowercase();
            let fixed = lower.replace("iso8859", "iso-8859").replace("microsoft-", "");
            encoding_rs::Encoding::for_label(fixed.as_bytes())
        })
        .unwrap_or(encoding_rs::WINDOWS_1252);
    encoding.decode_without_bom_handling(bytes).0.into_owned()
}

/// The files of a dictionary: the user's copy, else the bundled one.
/// (affix file, word list), decoded.
fn files(dir: &Path, name: &str) -> Option<(Option<String>, String)> {
    let aff_path = dir.join(format!("{name}.aff"));
    let dic_path = dir.join(format!("{name}.dic"));
    if valid_name(name)
        && let Ok(dic) = std::fs::read(&dic_path)
    {
        let aff = std::fs::read(&aff_path).ok();
        let text = decode(&dic, aff.as_deref());
        return Some((aff.map(|aff| decode(&aff, None)), text));
    }
    let (_, _, aff, dic) = BUNDLED.iter().find(|(known, ..)| *known == name)?;
    let aff = aff.map(inflate);
    let dic = inflate(dic);
    Some((aff.as_deref().map(|aff| decode(aff, None)), decode(&dic, aff.as_deref())))
}

/// The words of a `.dic` file: its lines after the count.
fn dic_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines().skip(1).map(str::trim).filter(|line| !line.is_empty())
}

/// The dictionary to check with: `main` with its affix rules, then the
/// words of `extra` and of the user's own list (LLSpellChecker::initHunspell,
/// which add_dic's the secondary dictionaries and user_custom.dic).
pub fn build(dir: &Path, main: &str, extra: &[String]) -> Result<spellbook::Dictionary, String> {
    let (aff, dic) = files(dir, main).ok_or_else(|| format!("no dictionary \"{main}\""))?;
    let aff = aff.ok_or_else(|| format!("\"{main}\" has no affix file"))?;
    let mut dict = parse(&aff, &dic).map_err(|e| format!("\"{main}\": {e}"))?;
    for name in extra.iter().filter(|name| *name != main) {
        let Some((_, words)) = files(dir, name) else {
            log::warn!("spelling: no extra dictionary \"{name}\"");
            continue;
        };
        add_all(&mut dict, dic_lines(&words));
    }
    let custom = read_words(&dir.join(CUSTOM_FILE));
    add_all(&mut dict, custom.iter().map(String::as_str));
    Ok(dict)
}

/// The word-list lines a dictionary may lose before it counts as broken.
const MAX_DROPPED_LINES: usize = 64;

/// A dictionary parsed as Hunspell reads it: a `.dic` line whose flags do
/// not parse (da_dk's `"A/S"`, a slash not escaped) is left out, where
/// spellbook alone refuses the whole file.
pub(super) fn parse(aff: &str, dic: &str) -> Result<spellbook::Dictionary, spellbook::ParseDictionaryError> {
    let mut dic = std::borrow::Cow::Borrowed(dic);
    let mut dropped = 0;
    loop {
        match spellbook::Dictionary::new(aff, &dic) {
            Ok(dict) => return Ok(dict),
            Err(e)
                if matches!(e.source, spellbook::ParseDictionaryErrorSource::Dic)
                    && dropped < MAX_DROPPED_LINES
                    && let Some(line) = e.line_number.and_then(|n| n.checked_sub(1)) =>
            {
                let lines: Vec<&str> = dic.lines().collect();
                let Some(bad) = lines.get(line) else {
                    return Err(e);
                };
                log::info!("spelling: line {} left out ({e}): {bad}", line + 1);
                let kept: Vec<&str> = lines.iter().enumerate().filter(|(i, _)| *i != line).map(|(_, l)| *l).collect();
                dic = std::borrow::Cow::Owned(kept.join("\n"));
                dropped += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Adds the lines of a word list; a line whose flags do not parse with the
/// main affix rules still adds its word.
fn add_all<'a>(dict: &mut spellbook::Dictionary, lines: impl Iterator<Item = &'a str>) {
    for line in lines {
        if dict.add(line).is_err()
            && let Some((word, _)) = line.split_once('/')
        {
            let _ = dict.add(&word.replace('/', ""));
        }
    }
}

/// A Legacy word list: the count, then one word per line.
pub fn read_words(path: &Path) -> Vec<String> {
    match std::fs::read(path) {
        Ok(bytes) => dic_lines(&decode(&bytes, None)).map(str::to_owned).collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            log::warn!("spelling: {}: {e}", path.display());
            Vec::new()
        }
    }
}

/// LLSpellChecker::addToDictFile: the list written again with `word` added.
pub fn append_word(path: &Path, word: &str) {
    let mut words = read_words(path);
    if words.iter().any(|known| known == word) {
        return;
    }
    words.push(word.to_owned());
    let mut text = format!("{}\n", words.len());
    for word in &words {
        text.push_str(word);
        text.push('\n');
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = crate::atomic_file::write(path, text.as_bytes()) {
        log::warn!("spelling: {}: {e}", path.display());
    }
}

pub fn custom_path(dir: &Path) -> PathBuf {
    dir.join(CUSTOM_FILE)
}

pub fn ignore_path(dir: &Path) -> PathBuf {
    dir.join(IGNORE_FILE)
}

/// A dictionary file chosen on disk (`.dic`, with the `.aff` beside it if
/// any) copied into the user's folder and listed, under its file name; the
/// language shown is that name. Its entry, or why not.
pub fn import(dir: &Path, dic: &Path) -> Result<Info, String> {
    let name = dic
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| valid_name(s))
        .ok_or_else(|| format!("{}: not a dictionary name", dic.display()))?;
    let words = std::fs::read(dic).map_err(|e| format!("{}: {e}", dic.display()))?;
    let aff = std::fs::read(dic.with_extension("aff")).ok();
    // a main dictionary must parse; an extra one is only a word list
    if let Some(aff) = &aff {
        parse(&decode(aff, None), &decode(&words, Some(aff))).map_err(|e| format!("{name}: {e}"))?;
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    crate::atomic_file::write(&dir.join(format!("{name}.dic")), &words).map_err(|e| e.to_string())?;
    if let Some(aff) = &aff {
        crate::atomic_file::write(&dir.join(format!("{name}.aff")), aff).map_err(|e| e.to_string())?;
    }
    let info = Info {
        language: dic.file_stem().and_then(|s| s.to_str()).unwrap_or(&name).to_owned(),
        name,
        primary: aff.is_some(),
        imported: true,
    };
    let mut list = imported(dir);
    list.retain(|known| known.name != info.name);
    list.push(info.clone());
    save_imported(dir, &list)?;
    Ok(info)
}

/// LLSpellChecker::removeDictionary: an imported dictionary's files and
/// entry.
pub fn remove(dir: &Path, name: &str) -> Result<(), String> {
    let mut list = imported(dir);
    if !valid_name(name) || !list.iter().any(|info| info.name == name) {
        return Err(format!("\"{name}\" was not imported"));
    }
    for ext in ["dic", "aff"] {
        let path = dir.join(format!("{name}.{ext}"));
        if let Err(e) = std::fs::remove_file(&path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(format!("{}: {e}", path.display()));
        }
    }
    list.retain(|info| info.name != name);
    save_imported(dir, &list)
}

fn save_imported(dir: &Path, list: &[Info]) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(list).map_err(|e| e.to_string())?;
    crate::atomic_file::write(&dir.join(IMPORTED_FILE), &json).map_err(|e| e.to_string())
}
