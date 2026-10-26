//! Spell checking of the text fields, like the Legacy Viewer's
//! LLSpellChecker (indra/llui/llspellcheck.cpp, originally LGPL 2.1) and its
//! SpellCheck / SpellCheckDictionary settings: a main Hunspell dictionary
//! with its affix rules, extra word lists (the Second Life glossary by
//! default), the words the user added and the ones they ignore.
//!
//! Hunspell is replaced by spellbook, a Rust reader of the same
//! dictionaries. Everything slow runs on one spelling thread: inflating and
//! parsing a dictionary (a fraction of a second for the large ones) and the
//! suggestions, prepared as soon as a word is found misspelled, so the text
//! menu opens with them at once. The fields check their words on the
//! interface thread, a cached verdict per word, so only the words typed
//! since the last frame reach the dictionary (microseconds each).
//!
//! `ui/spelling.rs` draws the squiggles and `ui/text_menu.rs` the
//! suggestions, Add to dictionary and Ignore.

pub mod dictionaries;
pub mod words;

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, mpsc};

pub use dictionaries::Info;

/// The Legacy menu's five suggestion entries (menu_text_editor.xml).
pub const MAX_SUGGESTIONS: usize = 5;

/// Preferences › Chat › Spelling, with the Legacy Viewer's defaults
/// (SpellCheck on, SpellCheckDictionary "English (United States),Second
/// Life Glossary").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpellSettings {
    /// SpellCheck.
    pub enabled: bool,
    /// The main dictionary's name (the first of SpellCheckDictionary).
    pub main: String,
    /// The extra dictionaries' names (the others).
    pub extra: Vec<String>,
}

impl Default for SpellSettings {
    fn default() -> Self {
        SpellSettings {
            enabled: true,
            main: "en_us".to_owned(),
            extra: vec![dictionaries::GLOSSARY.to_owned()],
        }
    }
}

/// Where the spell checker stands, for the preferences.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Off,
    Loading,
    Ready,
    /// The dictionary could not be read (the message is for the log and
    /// the tooltip).
    Failed(String),
}

enum Job {
    Load { generation: u64, settings: SpellSettings },
    Suggest { generation: u64, word: String },
    Add { word: String },
    Ignore { word: String },
    Import { dic: PathBuf },
    Remove { name: String },
}

#[derive(Default)]
struct State {
    /// The settings asked for, and the generation of that request: a
    /// dictionary built for an older one is dropped.
    wanted: Option<SpellSettings>,
    generation: u64,
    /// Changes with every verdict that may change (a dictionary ready, a
    /// word added or ignored): the fields check their text again.
    revision: u64,
    dict: Option<Arc<RwLock<spellbook::Dictionary>>>,
    status: Status,
    /// word → spelled right, for the current dictionary.
    verdicts: HashMap<String, bool>,
    /// Misspelled word → its suggestions, once the spelling thread has them.
    suggestions: HashMap<String, Vec<String>>,
    /// Words sent to the spelling thread for suggestions.
    asked: HashSet<String>,
    /// LLSpellChecker::mIgnoreList, lower case.
    ignore: HashSet<String>,
    catalogue: Option<Arc<Vec<Info>>>,
    /// The result of the last import, for the preferences.
    import: Option<Result<String, String>>,
    worker: Option<mpsc::Sender<Job>>,
    ctx: Option<egui::Context>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Mutex::default);

/// Above this many cached verdicts the cache starts again (a long session
/// of typing in many languages).
const MAX_VERDICTS: usize = 50_000;

impl State {
    fn send(&mut self, job: Job) {
        if self.worker.is_none() {
            let (tx, rx) = mpsc::channel();
            let spawned = std::thread::Builder::new().name("aurora-spelling".into()).spawn(move || worker(rx));
            match spawned {
                Ok(_) => self.worker = Some(tx),
                Err(e) => {
                    log::warn!("spelling: no thread: {e}");
                    return;
                }
            }
        }
        if let Some(worker) = &self.worker
            && worker.send(job).is_err()
        {
            self.worker = None;
        }
    }

    fn repaint(&self) {
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint();
        }
    }

    /// A new dictionary or new words: every verdict again.
    fn reset_verdicts(&mut self) {
        self.revision += 1;
        self.verdicts.clear();
        self.suggestions.clear();
        self.asked.clear();
        self.repaint();
    }
}

/// Installs the settings (at start and whenever they change): a dictionary
/// different from the loaded one is built on the spelling thread, the old
/// one checking until it is ready.
pub fn configure(ctx: &egui::Context, settings: &SpellSettings) {
    let mut st = STATE.lock();
    st.ctx = Some(ctx.clone());
    if st.wanted.as_ref() == Some(settings) {
        return;
    }
    st.wanted = Some(settings.clone());
    st.generation += 1;
    if !settings.enabled {
        st.dict = None;
        st.status = Status::Off;
        st.reset_verdicts();
        return;
    }
    st.status = Status::Loading;
    let generation = st.generation;
    st.send(Job::Load {
        generation,
        settings: settings.clone(),
    });
}

/// The fields can check their words.
pub fn active() -> bool {
    STATE.lock().dict.is_some()
}

pub fn status() -> Status {
    STATE.lock().status.clone()
}

/// Changes with every new dictionary or word added or ignored: the fields
/// check their text again.
pub fn revision() -> u64 {
    STATE.lock().revision
}

/// The misspelled words of `text`: their character ranges and themselves.
/// Their suggestions are asked at once.
pub fn misspelled(text: &str) -> Vec<(Range<usize>, String)> {
    let found = words::words(text);
    if found.is_empty() {
        return Vec::new();
    }
    let mut st = STATE.lock();
    let Some(dict) = st.dict.clone() else {
        return Vec::new();
    };
    if st.verdicts.len() > MAX_VERDICTS {
        st.verdicts.clear();
    }
    let mut out = Vec::new();
    let mut dict_guard = None;
    for word in found {
        let w = &text[word.bytes.clone()];
        let right = match st.verdicts.get(w) {
            Some(&right) => right,
            None => {
                let dict = dict_guard.get_or_insert_with(|| dict.read());
                let right = st.ignore.contains(&w.to_lowercase()) || dict.check(w) || dict.check(&w.replace('\u{2019}', "'"));
                st.verdicts.insert(w.to_owned(), right);
                right
            }
        };
        if !right {
            out.push((word.chars, w.to_owned()));
        }
    }
    drop(dict_guard);
    let generation = st.generation;
    for (_, word) in &out {
        if !st.asked.contains(word) {
            st.asked.insert(word.clone());
            st.send(Job::Suggest {
                generation,
                word: word.clone(),
            });
        }
    }
    out
}

/// The suggestions for a misspelled word, at most [`MAX_SUGGESTIONS`];
/// `None` while the spelling thread looks for them.
pub fn suggestions(word: &str) -> Option<Vec<String>> {
    let mut st = STATE.lock();
    if let Some(found) = st.suggestions.get(word) {
        return Some(found.clone());
    }
    if st.dict.is_none() {
        return Some(Vec::new());
    }
    if st.asked.insert(word.to_owned()) {
        let generation = st.generation;
        st.send(Job::Suggest {
            generation,
            word: word.to_owned(),
        });
    }
    None
}

/// LLSpellChecker::addToCustomDictionary: the word is right from now on,
/// in user_custom.dic.
pub fn add_to_dictionary(word: &str) {
    let mut st = STATE.lock();
    st.verdicts.insert(word.to_owned(), true);
    st.send(Job::Add { word: word.to_owned() });
    st.repaint();
}

/// LLSpellChecker::addToIgnoreList: the word, in any case, is never
/// marked again (user_ignore.dic).
pub fn ignore(word: &str) {
    let lower = word.to_lowercase();
    let mut st = STATE.lock();
    st.ignore.insert(lower.clone());
    st.verdicts.retain(|known, _| known.to_lowercase() != lower);
    st.revision += 1;
    st.send(Job::Ignore { word: lower });
    st.repaint();
}

/// The bundled and imported dictionaries (read once, again after an
/// import or a removal).
pub fn catalogue() -> Arc<Vec<Info>> {
    if let Some(list) = STATE.lock().catalogue.clone() {
        return list;
    }
    let list = Arc::new(dictionaries::catalogue(&dictionaries::user_dir()));
    STATE.lock().catalogue = Some(list.clone());
    list
}

/// "Import…": a `.dic` file chosen on disk, copied into the user's folder
/// on the spelling thread (LLFloaterSpellCheckerImport).
pub fn import() {
    let title = aurora_i18n::tr!("spelling-import-title");
    let filter = aurora_i18n::tr!("spelling-import-filter");
    let spawned = std::thread::Builder::new().name("aurora-dictionary-dialog".into()).spawn(move || {
        if let Some(dic) = rfd::FileDialog::new().set_title(title).add_filter(filter, &["dic"]).pick_file() {
            STATE.lock().send(Job::Import { dic });
        }
    });
    if let Err(e) = spawned {
        log::warn!("spelling: no dialog thread: {e}");
    }
}

/// The last import's dictionary name or error, taken once.
pub fn take_import_result() -> Option<Result<String, String>> {
    STATE.lock().import.take()
}

/// LLSpellChecker::removeDictionary (an imported dictionary not in use).
pub fn remove(name: &str) {
    STATE.lock().send(Job::Remove { name: name.to_owned() });
}

fn worker(rx: mpsc::Receiver<Job>) {
    let dir = dictionaries::user_dir();
    {
        let ignore: HashSet<String> = dictionaries::read_words(&dictionaries::ignore_path(&dir))
            .into_iter()
            .map(|w| w.to_lowercase())
            .collect();
        let mut st = STATE.lock();
        st.ignore.extend(ignore);
        st.verdicts.clear();
    }
    let mut suggester_dict: Option<(u64, Arc<RwLock<spellbook::Dictionary>>)> = None;
    while let Ok(job) = rx.recv() {
        match job {
            Job::Load { generation, settings } => {
                // only the last of the loads waiting
                if STATE.lock().generation != generation {
                    continue;
                }
                let started = std::time::Instant::now();
                let built = dictionaries::build(&dir, &settings.main, &settings.extra);
                let mut st = STATE.lock();
                if st.generation != generation {
                    continue;
                }
                match built {
                    Ok(dict) => {
                        log::info!(
                            "spelling: {} + {:?} ready in {} ms",
                            settings.main,
                            settings.extra,
                            started.elapsed().as_millis()
                        );
                        let dict = Arc::new(RwLock::new(dict));
                        st.dict = Some(dict.clone());
                        st.status = Status::Ready;
                        suggester_dict = Some((generation, dict));
                    }
                    Err(e) => {
                        log::warn!("spelling: {e}");
                        st.dict = None;
                        st.status = Status::Failed(e);
                    }
                }
                st.reset_verdicts();
            }
            Job::Suggest { generation, word } => {
                let Some(dict) = suggester_dict.as_ref().filter(|(g, _)| *g == generation).map(|(_, d)| d.clone()) else {
                    // a newer dictionary is coming: asked again by the fields
                    STATE.lock().asked.remove(&word);
                    continue;
                };
                let mut found = Vec::new();
                dict.read().suggest(&word, &mut found);
                found.truncate(MAX_SUGGESTIONS);
                let mut st = STATE.lock();
                if st.generation == generation {
                    st.suggestions.insert(word, found);
                    st.repaint();
                }
            }
            Job::Add { word } => {
                if let Some((_, dict)) = &suggester_dict
                    && let Err(e) = dict.write().add(&word)
                {
                    log::warn!("spelling: \"{word}\": {e}");
                }
                dictionaries::append_word(&dictionaries::custom_path(&dir), &word);
                // the case forms the dictionary now accepts too
                STATE.lock().reset_verdicts();
            }
            Job::Ignore { word } => dictionaries::append_word(&dictionaries::ignore_path(&dir), &word),
            Job::Import { dic } => {
                let result = dictionaries::import(&dir, &dic);
                if let Err(e) = &result {
                    log::warn!("spelling: import: {e}");
                }
                let mut st = STATE.lock();
                st.import = Some(result.map(|info| info.language));
                st.catalogue = None;
                // a dictionary in use with that name is read again
                if let Some(wanted) = st.wanted.clone() {
                    st.wanted = None;
                    let ctx = st.ctx.clone();
                    drop(st);
                    if let Some(ctx) = ctx {
                        configure(&ctx, &wanted);
                    }
                } else {
                    st.repaint();
                }
            }
            Job::Remove { name } => {
                if let Err(e) = dictionaries::remove(&dir, &name) {
                    log::warn!("spelling: remove: {e}");
                }
                let mut st = STATE.lock();
                st.catalogue = None;
                st.repaint();
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
