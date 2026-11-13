//! The LSL keywords of the script editor: the bundled catalog (taken from
//! keywords_lsl_default.xml), or the one the simulator names in its
//! SimulatorFeatures (LSLSyntaxId) and serves through LSLSyntax.
//!
//! Port of LLSyntaxIdLSL's switch between the default file and the
//! region's (indra/newview/llsyntaxid.cpp, originally LGPL 2.1): a region
//! without a syntax id uses the default, one with an id gets its own,
//! fetched once and cached (aurora_net::lsl_syntax). The highlighter and the
//! reference read the current catalog; each catalog lives for the whole
//! session (one per syntax id met, a few hundred kilobytes) so their entries
//! can be handed out as `&'static`.

use parking_lot::{Mutex, RwLock};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

/// The section a keyword comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Control,
    Type,
    Event,
    Function,
    Constant,
}

/// A keyword's signature and help (LLScriptEdCore::processKeywords).
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub group: String,
    #[serde(rename = "type")]
    pub data_type: String,
    pub signature: String,
    pub tooltip: String,
}

pub struct Catalog {
    /// The syntax id it came from; None for the bundled one.
    pub id: Option<Uuid>,
    words: HashMap<String, Group>,
    /// Sorted by name.
    entries: Vec<Entry>,
}

impl Catalog {
    pub fn word(&self, name: &str) -> Option<Group> {
        self.words.get(name).copied()
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn entry(&self, name: &str) -> Option<&Entry> {
        self.entries
            .binary_search_by(|e| e.name.as_str().cmp(name))
            .ok()
            .map(|i| &self.entries[i])
    }

    /// A simulator's keywords (aurora_net::lsl_syntax::keywords, sorted).
    pub fn from_keywords(id: Uuid, keywords: Vec<aurora_net::lsl_syntax::Keyword>) -> Catalog {
        let (words, entries) = Self::collect(keywords);
        Catalog::with_extras(Some(id), words, entries)
    }

    /// The bundled SLua keywords: keywords_lua_default.xml of
    /// secondlife/lsl-definitions, the file LLSyntaxDefCache falls back to
    /// (FILENAME_DEFAULT_LUA, indra/newview/llsyntaxid.cpp). No LSL extras.
    fn lua() -> Catalog {
        // Invariant: a bundled, tested build-time asset, not input.
        let doc = aurora_llsd::from_xml(include_bytes!("../assets/slua/keywords_lua_default.xml")).expect("valid bundled SLua keywords");
        let keywords = aurora_net::lsl_syntax::keywords(&doc).expect("bundled SLua keywords of the expected version");
        let (words, entries) = Self::collect(keywords);
        Catalog { id: None, words, entries }
    }

    /// A simulator's SLua keywords (ScriptDefinitions' lua_keywords.xml).
    pub fn lua_from_keywords(id: Uuid, keywords: Vec<aurora_net::lsl_syntax::Keyword>) -> Catalog {
        let (words, entries) = Self::collect(keywords);
        Catalog {
            id: Some(id),
            words,
            entries,
        }
    }

    fn collect(keywords: Vec<aurora_net::lsl_syntax::Keyword>) -> (HashMap<String, Group>, Vec<Entry>) {
        use aurora_net::lsl_syntax::Group as G;
        let mut words = HashMap::with_capacity(keywords.len());
        let mut entries = Vec::with_capacity(keywords.len());
        for k in keywords {
            let group = match k.group {
                G::Control => Group::Control,
                G::Type => Group::Type,
                G::Event => Group::Event,
                G::Function => Group::Function,
                G::Constant => Group::Constant,
            };
            words.insert(k.name.clone(), group);
            entries.push(Entry {
                group: k.group.name().to_owned(),
                name: k.name,
                data_type: k.data_type,
                signature: k.signature,
                tooltip: k.tooltip,
            });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        (words, entries)
    }

    /// The bundled catalog.
    fn bundled() -> Catalog {
        #[derive(Deserialize)]
        struct Words {
            controls: Vec<String>,
            types: Vec<String>,
            events: Vec<String>,
            functions: Vec<String>,
            constants: Vec<String>,
        }
        // Invariant: these JSON files are bundled, tested build-time assets, not input.
        let catalog: Words = serde_json::from_str(include_str!("../assets/lsl/keywords.json")).expect("valid bundled LSL keyword catalog");
        let entries: Vec<Entry> = serde_json::from_str(include_str!("../assets/lsl/reference.json")).expect("valid bundled LSL reference");
        let mut words = HashMap::new();
        for (names, group) in [
            (catalog.controls, Group::Control),
            (catalog.types, Group::Type),
            (catalog.events, Group::Event),
            (catalog.functions, Group::Function),
            (catalog.constants, Group::Constant),
        ] {
            words.extend(names.into_iter().map(|name| (name, group)));
        }
        Catalog::with_extras(None, words, entries)
    }

    /// The words of the language itself that no catalog lists: `default`
    /// and the preprocessor's switch statement.
    fn with_extras(id: Option<Uuid>, mut words: HashMap<String, Group>, entries: Vec<Entry>) -> Catalog {
        for name in ["default", "switch", "case", "break"] {
            words.insert(name.into(), Group::Control);
        }
        Catalog { id, words, entries }
    }
}

fn bundled() -> &'static Catalog {
    static BUNDLED: OnceLock<Catalog> = OnceLock::new();
    BUNDLED.get_or_init(Catalog::bundled)
}

/// Which catalog is in use, and the simulators' catalogs met this session.
#[derive(Default)]
struct Registry {
    /// None: the bundled catalog.
    current: RwLock<Option<&'static Catalog>>,
    /// By syntax id.
    met: Mutex<HashMap<Uuid, &'static Catalog>>,
    /// Bumped on each change, for the caches built from a catalog.
    generation: AtomicU64,
}

impl Registry {
    fn current(&self) -> &'static Catalog {
        self.current.read().unwrap_or_else(bundled)
    }

    fn current_or(&self, fallback: fn() -> &'static Catalog) -> &'static Catalog {
        self.current.read().unwrap_or_else(fallback)
    }

    fn use_known(&self, id: Uuid) -> bool {
        let known = self.met.lock().get(&id).copied();
        if let Some(c) = known {
            self.set(Some(c));
        }
        known.is_some()
    }

    fn install(&self, catalog: Catalog) {
        let id = catalog.id;
        let leaked: &'static Catalog = Box::leak(Box::new(catalog));
        if let Some(id) = id {
            self.met.lock().insert(id, leaked);
        }
        self.set(Some(leaked));
    }

    fn set(&self, catalog: Option<&'static Catalog>) {
        let mut current = self.current.write();
        let same = match (*current, catalog) {
            (None, None) => true,
            (Some(a), Some(b)) => std::ptr::eq(a, b),
            _ => false,
        };
        if !same {
            *current = catalog;
            self.generation.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Registry::default)
}

/// The regions' SLua catalogs, by the same syntax ids.
fn lua_registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Registry::default)
}

/// The bundled SLua catalog, parsed on first use (about 700 KB of XML).
fn bundled_lua() -> &'static Catalog {
    static LUA: OnceLock<Catalog> = OnceLock::new();
    LUA.get_or_init(Catalog::lua)
}

/// Reads the bundled SLua catalog on a background thread (about 8 ms in a
/// development build), once: the first SLua script then opens without a
/// hitch. A frame that needs it before then waits for the same reading.
pub fn warm_lua() {
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !STARTED.swap(true, Ordering::Relaxed) {
        std::thread::spawn(|| {
            bundled_lua();
        });
    }
}

/// The SLua catalog in use: the region's, else the bundled one.
pub fn lua() -> &'static Catalog {
    lua_registry().current_or(bundled_lua)
}

/// The catalog in use.
pub fn current() -> &'static Catalog {
    registry().current()
}

/// Changes each time a catalog in use (LSL or SLua) changes.
pub fn generation() -> u64 {
    registry().generation.load(Ordering::Relaxed) + lua_registry().generation.load(Ordering::Relaxed)
}

/// The syntax id in use (None: the bundled catalog).
pub fn current_id() -> Option<Uuid> {
    current().id
}

/// Switch to the catalogs of `id` if it was met this session; false when it
/// still has to be fetched. A syntax that gave no SLua keywords uses the
/// bundled ones.
pub fn use_known(id: Uuid) -> bool {
    let known = registry().use_known(id);
    if known && !lua_registry().use_known(id) {
        lua_registry().set(None);
    }
    known
}

/// Use a simulator's catalogs (kept for the rest of the session); without
/// SLua keywords, the bundled ones (LLSyntaxDefCache keeps the defaults of
/// an omitted language).
pub fn install(catalog: Catalog, lua: Option<Catalog>) {
    registry().install(catalog);
    match lua {
        Some(lua) => lua_registry().install(lua),
        None => lua_registry().set(None),
    }
}

/// Back to the bundled catalogs (a region without a syntax id).
pub fn use_bundled() {
    registry().set(None);
    lua_registry().set(None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::lsl_syntax::{Group as G, Keyword};

    #[test]
    fn the_bundled_catalog_is_sorted_and_complete() {
        let c = bundled();
        assert!(c.entries().len() > 1100);
        assert!(c.entries().windows(2).all(|w| w[0].name < w[1].name));
        assert_eq!(c.word("rotation"), Some(Group::Type));
        assert_eq!(c.word("default"), Some(Group::Control));
        assert_eq!(
            c.entry("llSay").map(|e| e.signature.as_str()),
            Some("void llSay(integer Channel, string Text)")
        );
        assert_eq!(c.id, None);
    }

    #[test]
    fn the_slua_catalog_keeps_library_names_and_no_lsl_words() {
        let started = std::time::Instant::now();
        let c = Catalog::lua();
        eprintln!("SLua catalog parsed in {:?}", started.elapsed());
        assert!(c.entries().len() > 1500);
        assert!(c.entries().windows(2).all(|w| w[0].name < w[1].name));
        assert_eq!(c.word("ll.Say"), Some(Group::Function));
        assert_eq!(c.word("local"), Some(Group::Control));
        assert_eq!(c.word("nil"), Some(Group::Constant));
        assert_eq!(c.word("default"), None, "no LSL state");
        assert_eq!(c.word("llSay"), None);
        assert!(c.entry("ll.Say").is_some_and(|e| e.signature.contains("ll.Say(")));
    }

    #[test]
    fn a_region_slua_catalog_replaces_the_bundled_one_and_returns() {
        let r = Registry::default();
        assert!(std::ptr::eq(r.current_or(bundled_lua), bundled_lua()));
        let id = Uuid::from_u128(0x1a);
        let zeta = Keyword {
            name: "ll.Zeta".into(),
            group: G::Function,
            data_type: String::new(),
            signature: "void ll.Zeta()".into(),
            tooltip: String::new(),
        };
        r.install(Catalog::lua_from_keywords(id, vec![zeta]));
        let region = r.current_or(bundled_lua);
        assert_eq!((region.id, region.word("ll.Zeta")), (Some(id), Some(Group::Function)));
        assert_eq!(region.word("default"), None, "no LSL extras");
        r.set(None);
        assert!(std::ptr::eq(r.current_or(bundled_lua), bundled_lua()));
        assert!(r.use_known(id));
    }

    #[test]
    fn a_simulator_catalog_replaces_and_returns() {
        let keyword = |name: &str, group| Keyword {
            name: name.into(),
            group,
            data_type: String::new(),
            signature: format!("void {name}()"),
            tooltip: String::new(),
        };
        let id = Uuid::from_u128(0x5e1);
        let catalog = Catalog::from_keywords(id, vec![keyword("llZeta", G::Function), keyword("llAlpha", G::Function)]);
        assert_eq!(catalog.entries()[0].name, "llAlpha");
        assert_eq!(catalog.entry("llZeta").map(|e| e.group.as_str()), Some("functions"));
        assert_eq!(catalog.word("switch"), Some(Group::Control));

        // a registry of its own: the other tests read the global one
        let r = Registry::default();
        assert!(!r.use_known(id));
        r.install(catalog);
        assert_eq!(r.current().id, Some(id));
        assert_eq!(r.current().word("llSay"), None);
        assert_eq!(r.generation.load(Ordering::Relaxed), 1);
        r.set(None);
        assert_eq!(r.current().id, None);
        assert!(r.use_known(id));
        assert_eq!(r.current().id, Some(id));
        r.set(r.met.lock().get(&id).copied());
        assert_eq!(r.generation.load(Ordering::Relaxed), 3, "the same catalog again is no change");
    }
}
