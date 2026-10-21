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
        Catalog::with_extras(Some(id), words, entries)
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

/// The catalog in use.
pub fn current() -> &'static Catalog {
    registry().current()
}

/// Changes each time the catalog in use changes.
pub fn generation() -> u64 {
    registry().generation.load(Ordering::Relaxed)
}

/// The syntax id in use (None: the bundled catalog).
pub fn current_id() -> Option<Uuid> {
    current().id
}

/// Switch to the catalog of `id` if it was met this session; false when it
/// still has to be fetched.
pub fn use_known(id: Uuid) -> bool {
    registry().use_known(id)
}

/// Use a simulator's catalog (kept for the rest of the session).
pub fn install(catalog: Catalog) {
    registry().install(catalog);
}

/// Back to the bundled catalog (a region without a syntax id).
pub fn use_bundled() {
    registry().set(None);
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
