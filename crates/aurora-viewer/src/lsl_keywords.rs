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
    /// The language deprecated it (`assets/lsl/definitions.json`).
    #[serde(default)]
    pub deprecated: bool,
    /// The name to use instead, when the language gives one (LSL `llSay`,
    /// SLua `ll.Say`).
    #[serde(default)]
    pub replaced_by: Option<String>,
    /// The parameters, as the definitions write them (`channel`, not
    /// `Channel`), with what each one means. Empty when the definitions and
    /// the catalog do not agree on how many there are.
    #[serde(skip)]
    pub arguments: Vec<Argument>,
}

/// One parameter of a call: its name and what it means.
#[derive(Clone, Debug, PartialEq)]
pub struct Argument {
    pub name: String,
    pub help: String,
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
        let (words, entries) = Self::collect(keywords, false);
        Catalog::with_extras(Some(id), words, entries)
    }

    /// The bundled SLua keywords: keywords_lua_default.xml of
    /// secondlife/lsl-definitions, the file LLSyntaxDefCache falls back to
    /// (FILENAME_DEFAULT_LUA, indra/newview/llsyntaxid.cpp). No LSL extras.
    fn lua() -> Catalog {
        // Invariant: a bundled, tested build-time asset, not input.
        let doc = aurora_llsd::from_xml(BUNDLED_LUA_XML).expect("valid bundled SLua keywords");
        let keywords = aurora_net::lsl_syntax::keywords(&doc).expect("bundled SLua keywords of the expected version");
        let (words, entries) = Self::collect(keywords, true);
        Catalog { id: None, words, entries }
    }

    /// A simulator's SLua keywords (ScriptDefinitions' lua_keywords.xml).
    pub fn lua_from_keywords(id: Uuid, keywords: Vec<aurora_net::lsl_syntax::Keyword>) -> Catalog {
        let (words, entries) = Self::collect(keywords, true);
        Catalog {
            id: Some(id),
            words,
            entries,
        }
    }

    fn collect(keywords: Vec<aurora_net::lsl_syntax::Keyword>, lua: bool) -> (HashMap<String, Group>, Vec<Entry>) {
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
                deprecated: k.deprecated,
                replaced_by: None,
                arguments: Vec::new(),
            });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        apply_arguments(&mut entries);
        apply_deprecations(&mut entries, lua);
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
        let mut entries: Vec<Entry> =
            serde_json::from_str(include_str!("../assets/lsl/reference.json")).expect("valid bundled LSL reference");
        apply_arguments(&mut entries);
        apply_deprecations(&mut entries, false);
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

/// What the definitions say of the names the catalogs already have: the
/// deprecated ones and what they use instead (None for one without a
/// replacement), and each function's or event's parameters.
#[derive(Deserialize)]
struct Definitions {
    #[serde(default)]
    deprecated: HashMap<String, Option<String>>,
    #[serde(default)]
    arguments: HashMap<String, Vec<(String, String)>>,
}

/// The bundled table (assets/lsl/definitions.json), read once.
fn definitions() -> &'static Definitions {
    static TABLE: OnceLock<Definitions> = OnceLock::new();
    TABLE.get_or_init(|| {
        // Invariant: a bundled, tested build-time asset, not input.
        serde_json::from_str(include_str!("../assets/lsl/definitions.json")).expect("valid bundled LSL definitions")
    })
}

/// How a script writes a function's name: LSL `llSay`, SLua `ll.Say`. The
/// definitions table keys its rows by the first, and names the calls to use
/// instead by the second; constants and events read the same in both.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Spelling {
    Lsl,
    Lua,
}

impl Spelling {
    /// The spelling a script of `lua`'s language reads.
    fn of(lua: bool) -> Self {
        if lua { Spelling::Lua } else { Spelling::Lsl }
    }
}

/// `name` in `spelling`, whichever spelling it was read in: `ll.Say` and
/// `llSay` are the same function, and a name that is not one (a constant, an
/// event) is left as it is.
fn name_in(name: &str, spelling: Spelling) -> String {
    let Some(function) = function_of(name) else {
        return name.to_owned();
    };
    match spelling {
        Spelling::Lsl => format!("ll{function}"),
        Spelling::Lua => format!("ll.{function}"),
    }
}

/// What `name` says after its `ll`, when it is a function's name: `ll.Say`
/// and `llSay`, whose third character is a capital in the second.
fn function_of(name: &str) -> Option<&str> {
    let rest = match name.strip_prefix("ll.") {
        Some(rest) => rest,
        None => name
            .strip_prefix("ll")
            .filter(|rest| rest.as_bytes().first().is_some_and(u8::is_ascii_uppercase))?,
    };
    (!rest.is_empty()).then_some(rest)
}

/// Marks the entries the language has deprecated, and names the one to use
/// instead. The definitions are newer or older than a region; only the names a
/// catalog already has are decorated, never added.
fn apply_deprecations(entries: &mut [Entry], lua: bool) {
    let table = &definitions().deprecated;
    for entry in entries {
        let key = name_in(&entry.name, Spelling::Lsl);
        let Some(use_instead) = table.get(&key) else {
            continue;
        };
        entry.deprecated = true;
        // the table names the call to use instead in its own spelling
        entry.replaced_by = use_instead.as_ref().map(|name| name_in(name, Spelling::of(lua)));
    }
}

/// Renames each entry's parameters as the definitions write them (`channel`,
/// not `Channel`) and keeps what each one means, when both write the same
/// number of them. A region whose syntax is newer or older keeps its own
/// names and gets no help, rather than the wrong one.
fn apply_arguments(entries: &mut [Entry]) {
    let table = &definitions().arguments;
    for entry in entries {
        let key = name_in(&entry.name, Spelling::Lsl);
        let Some(arguments) = table.get(&key) else {
            continue;
        };
        let params = signature_parameters(&entry.signature);
        if arguments.is_empty() || params.len() != arguments.len() {
            continue;
        }
        if arguments
            .iter()
            .any(|(name, _)| name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        {
            continue;
        }
        let Some(signature) = renamed(&entry.signature, &params, arguments) else {
            continue;
        };
        entry.signature = signature;
        entry.arguments = arguments
            .iter()
            .map(|(name, help)| Argument {
                name: name.clone(),
                help: help.clone(),
            })
            .collect();
    }
}

/// A signature with each parameter's name replaced, its type kept: the
/// definitions write `channel` where the Legacy Viewer's keywords write
/// `Channel`.
fn renamed(signature: &str, params: &[String], arguments: &[(String, String)]) -> Option<String> {
    let open = signature.find('(')?;
    let close = signature.rfind(')')?;
    if close <= open {
        return None;
    }
    let mut inside = String::new();
    for (param, (name, _)) in params.iter().zip(arguments) {
        let cut = param.rfind(|c: char| c.is_whitespace())?;
        if !inside.is_empty() {
            inside.push_str(", ");
        }
        inside.push_str(param[..cut].trim_end());
        inside.push(' ');
        inside.push_str(name);
    }
    Some(format!("{}({}){}", &signature[..open], inside, &signature[close + 1..]))
}

/// The parameters of a catalog signature (`void llSay(integer channel,
/// string msg)`), as written between its outermost brackets: the inlay hints
/// and the documentation pane read them here.
pub fn signature_parameters(signature: &str) -> Vec<String> {
    let Some(open) = signature.find('(') else {
        return Vec::new();
    };
    let Some(close) = signature.rfind(')') else {
        return Vec::new();
    };
    if close <= open {
        return Vec::new();
    }
    let mut params = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    for ch in signature[open + 1..close].chars() {
        match ch {
            '(' | '[' | '{' => {
                depth += 1;
                current.push(ch);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                let param = current.trim();
                if !param.is_empty() {
                    params.push(param.to_owned());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    let param = current.trim();
    if !param.is_empty() {
        params.push(param.to_owned());
    }
    params
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

/// keywords_lua_default.xml as bundled: the catalog's source, and the file
/// the editor sync serves when the region gives no SLua keywords.
pub const BUNDLED_LUA_XML: &[u8] = include_bytes!("../assets/slua/keywords_lua_default.xml");

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
            Some("void llSay(integer channel, string msg)")
        );
        assert_eq!(c.id, None);
    }

    /// The bundled table marks the calls the definitions deprecated, names
    /// the one to use instead, and leaves the others alone.
    #[test]
    fn the_catalog_says_what_a_deprecated_call_uses_instead() {
        let c = bundled();
        let old = c.entry("llSetPrimitiveParams").expect("a deprecated call");
        assert!(old.deprecated);
        assert_eq!(old.replaced_by.as_deref(), Some("llSetLinkPrimitiveParamsFast"));
        let cloud = c.entry("llCloud").expect("deprecated without a replacement");
        assert!(cloud.deprecated);
        assert_eq!(cloud.replaced_by, None);
        assert!(!c.entry("llSay").expect("a call in use").deprecated);
        assert_eq!(name_in("ll.PlaySound", Spelling::Lsl), "llPlaySound");
        assert_eq!(name_in("llPlaySound", Spelling::Lua), "ll.PlaySound");
        assert_eq!(name_in("ll.PlaySound", Spelling::Lua), "ll.PlaySound");
        assert_eq!(name_in("PRIM_FULLBRIGHT", Spelling::Lua), "PRIM_FULLBRIGHT");
        // either spelling reads the same function, and the constant is not one
        assert_eq!(name_in(&name_in("ll.PlaySound", Spelling::Lsl), Spelling::Lua), "ll.PlaySound");
        assert_eq!(name_in(&name_in("listen", Spelling::Lua), Spelling::Lsl), "listen");
        assert_eq!(function_of("ll"), None, "no function's name");

        // an SLua catalog names its functions ll.Sound; the table's llSound
        let mut lua = vec![Entry {
            name: "ll.Sound".into(),
            group: "functions".into(),
            data_type: String::new(),
            signature: "void ll.Sound(string Sound)".into(),
            tooltip: String::new(),
            deprecated: false,
            replaced_by: None,
            arguments: Vec::new(),
        }];
        apply_deprecations(&mut lua, true);
        assert!(lua[0].deprecated);
        assert_eq!(lua[0].replaced_by.as_deref(), Some("ll.PlaySound"));
    }

    /// The definitions name each parameter (`channel`, `msg`) and say what it
    /// means; a signature whose argument count disagrees keeps its own names
    /// and gets no help.
    #[test]
    fn the_catalog_names_arguments_as_the_definitions_do() {
        let c = bundled();
        let say = c.entry("llSay").expect("a call");
        assert_eq!(say.signature, "void llSay(integer channel, string msg)");
        assert_eq!(
            say.arguments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            ["channel", "msg"]
        );
        assert!(say.arguments[0].help.contains("Chat channel"));
        assert!(say.arguments[1].help.contains("broadcast"));

        let listen = c.entry("llListen").expect("a call");
        assert_eq!(
            listen.signature,
            "integer llListen(integer channel, string name, key id, string msg)"
        );
        assert_eq!(listen.arguments.len(), 4);

        // a signature of another number of arguments is left as it is
        let mut wrong = Entry {
            name: "llListen".into(),
            group: "functions".into(),
            data_type: String::new(),
            signature: "integer llListen(integer one)".into(),
            tooltip: String::new(),
            deprecated: false,
            replaced_by: None,
            arguments: Vec::new(),
        };
        apply_arguments(std::slice::from_mut(&mut wrong));
        assert_eq!(wrong.signature, "integer llListen(integer one)");
        assert!(wrong.arguments.is_empty());

        // an SLua name reads its LSL entry, `ll.Say` for `llSay`
        let mut lua = Entry {
            name: "ll.Say".into(),
            group: "functions".into(),
            data_type: String::new(),
            signature: "ll.Say(integer Channel, string Text)".into(),
            tooltip: String::new(),
            deprecated: false,
            replaced_by: None,
            arguments: Vec::new(),
        };
        apply_arguments(std::slice::from_mut(&mut lua));
        assert_eq!(lua.signature, "ll.Say(integer channel, string msg)");
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
            deprecated: false,
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
            deprecated: false,
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

    /// A signature's parameters, as the inlay hints and the documentation pane
    /// read them: each `type name`, and nothing between the brackets of one
    /// that takes none.
    #[test]
    fn a_signature_lists_its_parameters() {
        assert_eq!(
            signature_parameters("void llSay(integer Channel, string Text)"),
            ["integer Channel", "string Text"]
        );
        assert_eq!(signature_parameters("void llSleep(float sec)"), ["float sec"]);
        assert_eq!(
            signature_parameters("list llParseString2List(string src, list separators, list excludes)"),
            ["string src", "list separators", "list excludes"]
        );
        assert!(signature_parameters("integer llGetTime()").is_empty(), "no parameters");
    }
}
