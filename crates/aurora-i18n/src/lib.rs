//! Shared UI language and Fluent messages, including network notifications.
//! Selection follows LLUI::getUILanguage (indra/llui/llui.cpp): saved choice,
//! system language, then English. Aurora has no separate installer language.
//!
//! Besides the bundled catalogs, the user may load a catalog file of their
//! own (Preferences › Language and updates). It is read once at startup
//! (`install_user_catalog`), so a new file applies at the next launch. The
//! Legacy Viewer has no such choice: it is an Aurora addition, for
//! translators and for the languages Aurora does not ship yet.

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{LazyLock, OnceLock, RwLock};

pub use fluent_bundle::FluentValue;
include!(concat!(env!("OUT_DIR"), "/locales.rs"));

struct Catalog {
    code: &'static str,
    bundle: FluentBundle<FluentResource>,
    // Resolve plain messages once, so labels neither allocate nor lock in
    // the frame loop. Both catalogs live for the lifetime of the process.
    plain: HashMap<String, String>,
}

static CATALOGS: OnceLock<Vec<OnceLock<Catalog>>> = OnceLock::new();
/// The user's own catalog file, installed at most once per process.
static USER: OnceLock<Catalog> = OnceLock::new();
/// The selected catalog: an index of `SOURCES`, `USER_INDEX`, or
/// `usize::MAX` before the first selection.
static CURRENT: AtomicUsize = AtomicUsize::new(usize::MAX);
const USER_INDEX: usize = usize::MAX - 1;
static SYSTEM: OnceLock<String> = OnceLock::new();

/// The money symbol messages show for "L$" (Tea::sActiveCurrency): an
/// OpenSim grid's or region's own, "L$" on Second Life.
const LINDEN: &str = "L$";
static CURRENCY: RwLock<&'static str> = RwLock::new(LINDEN);
/// Another symbol than "L$" is in use: only then are messages rewritten.
static CUSTOM: AtomicBool = AtomicBool::new(false);
/// Plain messages with the symbol swapped (Tea::wrapCurrency), by message
/// and symbol: a symbol changes at login or on entering a region, so these
/// few strings live for the process like the catalogs.
static WRAPPED: LazyLock<RwLock<HashMap<(usize, &'static str), &'static str>>> = LazyLock::new(Default::default);
static SYMBOLS: RwLock<Vec<&'static str>> = RwLock::new(Vec::new());

/// Cache composite label tables independently for every bundled locale.
/// Switching languages selects another table without allocating per frame.
pub struct Localized<T>(OnceLock<Vec<OnceLock<T>>>);

impl<T> Default for Localized<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Localized<T> {
    pub const fn new() -> Self {
        Self(OnceLock::new())
    }

    pub fn get(&'static self, build: impl FnOnce() -> T) -> &'static T {
        current();
        // the user's catalog has the slot after the bundled ones: it may
        // share a bundled catalog's code, not its texts
        let index = match CURRENT.load(Ordering::Acquire) {
            USER_INDEX => SOURCES.len(),
            index => index,
        };
        self.0.get_or_init(|| (0..=SOURCES.len()).map(|_| OnceLock::new()).collect())[index].get_or_init(build)
    }
}

fn bundled(index: usize) -> &'static Catalog {
    let catalogs = CATALOGS.get_or_init(|| SOURCES.iter().map(|_| OnceLock::new()).collect());
    catalogs[index].get_or_init(|| {
        let (code, source) = SOURCES[index];
        let resource = FluentResource::try_new(source.to_owned()).expect("bundled Fluent syntax is checked by tests");
        assemble(code, resource, false).0
    })
}

/// A catalog from its parsed messages, and how many messages it holds. A
/// bundled catalog's keys are unique (the parity test); in the user's file
/// a repeated key keeps its last version.
fn assemble(code: &'static str, resource: FluentResource, user: bool) -> (Catalog, usize) {
    let locale = code.parse().expect("catalog codes are checked language identifiers");
    let keys: Vec<_> = resource
        .entries()
        .filter_map(|entry| match entry {
            fluent_syntax::ast::Entry::Message(message) => Some(message.id.name.to_owned()),
            _ => None,
        })
        .collect();
    let messages = keys.len();
    let mut bundle = FluentBundle::new_concurrent(vec![locale]);
    bundle.set_use_isolating(false);
    if user {
        bundle.add_resource_overriding(resource);
    } else {
        bundle.add_resource(resource).expect("bundled keys are unique");
    }
    let mut plain = HashMap::new();
    for key in keys {
        if let Some(pattern) = bundle.get_message(&key).and_then(|message| message.value()) {
            let mut errors = Vec::new();
            let text = bundle.format_pattern(pattern, None, &mut errors);
            if errors.is_empty() {
                plain.insert(key, text.into_owned());
            }
        }
    }
    (Catalog { code, bundle, plain }, messages)
}

/// Largest catalog file read: the bundled ones are about 1 MB.
pub const USER_CATALOG_MAX_BYTES: u64 = 16 * 1024 * 1024;

/// The user's catalog file, once installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserCatalog {
    /// Its language, from the file's name (`de.ftl`); "en" when the name is
    /// not a language.
    pub code: &'static str,
    /// Whether the file's name gave the language.
    pub named: bool,
    /// The messages it holds; the others are shown in English.
    pub messages: usize,
    /// The entries Fluent could not read, left out.
    pub errors: usize,
}

/// Why the user's catalog file was not installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserCatalogError {
    /// Nothing readable: not one message.
    Empty,
    /// A catalog file is installed already in this process.
    AlreadyInstalled,
}

/// The language of a catalog file from its name without the extension, as
/// for the bundled ones: a language identifier whose language is an ISO 639
/// code (`de`, `pt-BR`, `pt_BR`). None for any other name (`my-translation`).
pub fn user_catalog_code(stem: &str) -> Option<String> {
    let code = stem.trim().replace('_', "-");
    let language = code.split('-').next().unwrap_or_default();
    let iso = (2..=3).contains(&language.len()) && language.bytes().all(|b| b.is_ascii_alphabetic());
    // parsed as the bundle's own locale type, which fluent does not re-export
    let parses = || {
        code.parse()
            .map(|locale| FluentBundle::<FluentResource>::new_concurrent(vec![locale]))
            .is_ok()
    };
    (iso && !language.eq_ignore_ascii_case("und") && parses()).then_some(code)
}

/// Installs the user's catalog file (its name without the extension, and
/// its text), at most once per process. `select_with` can then choose it;
/// a message or a variable it lacks uses English, as for the bundled
/// catalogs. An entry Fluent cannot read is left out, not the whole file.
pub fn install_user_catalog(stem: &str, source: &str) -> Result<UserCatalog, UserCatalogError> {
    if USER.get().is_some() {
        return Err(UserCatalogError::AlreadyInstalled);
    }
    let (catalog, info) = user_catalog(stem, source)?;
    USER.set(catalog).map_err(|_| UserCatalogError::AlreadyInstalled)?;
    Ok(info)
}

/// The user's catalog, built without installing it.
fn user_catalog(stem: &str, source: &str) -> Result<(Catalog, UserCatalog), UserCatalogError> {
    let named = user_catalog_code(stem);
    let code: &'static str = match &named {
        // the bundled English's code: plural rules of English
        None => "en",
        // once per process, like the catalog itself
        Some(code) => Box::leak(code.clone().into_boxed_str()),
    };
    // a file saved by Notepad may start with a byte order mark
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let (resource, errors) = match FluentResource::try_new(source.to_owned()) {
        Ok(resource) => (resource, 0),
        Err((resource, errors)) => (resource, errors.len()),
    };
    let (catalog, messages) = assemble(code, resource, true);
    if messages == 0 {
        return Err(UserCatalogError::Empty);
    }
    let info = UserCatalog {
        code,
        named: named.is_some(),
        messages,
        errors,
    };
    Ok((catalog, info))
}

/// The user's catalog, when one is installed.
pub fn user_catalog_installed() -> bool {
    USER.get().is_some()
}

/// Whether the interface uses the user's catalog now.
pub fn user_catalog_selected() -> bool {
    current();
    CURRENT.load(Ordering::Acquire) == USER_INDEX
}

fn english() -> usize {
    SOURCES
        .iter()
        .position(|&(code, _)| code == "en")
        .expect("English fallback catalog is bundled")
}

fn index(code: &str) -> Option<usize> {
    language_index(code, SOURCES.iter().map(|&(code, _)| code))
}

fn language_index<'a>(code: &str, available: impl Iterator<Item = &'a str> + Clone) -> Option<usize> {
    let normalized = if code.contains('_') {
        std::borrow::Cow::Owned(code.replace('_', "-"))
    } else {
        std::borrow::Cow::Borrowed(code)
    };
    let find = |candidate: &str| available.clone().position(|code| code.eq_ignore_ascii_case(candidate));
    let mut candidate = normalized.as_ref();
    while let Some((parent, _)) = candidate.rsplit_once('-') {
        if let Some(index) = find(candidate) {
            return Some(index);
        }
        if parent.eq_ignore_ascii_case("zh") {
            let region = candidate.split_once('-')?.1;
            let script = match region.to_ascii_lowercase().as_str() {
                "tw" | "hk" | "mo" => "zh-Hant",
                "cn" | "sg" => "zh-Hans",
                _ => candidate,
            };
            return find(script).or_else(|| find(parent));
        }
        candidate = parent;
    }
    find(candidate).or_else(|| candidate.eq_ignore_ascii_case("zh").then(|| find("zh-Hans")).flatten())
}

/// Pure selection, independent from the machine's locale and saved files.
pub fn resolve_language<'a>(preference: &str, system: &str, available: &'a [&str]) -> &'a str {
    let requested = if preference.is_empty() || preference == "default" {
        system
    } else {
        preference
    };
    language_index(requested, available.iter().copied()).map_or("en", |index| available[index])
}

pub fn system_language() -> &'static str {
    SYSTEM.get_or_init(|| sys_locale::get_locale().unwrap_or_else(|| "en".to_owned()))
}

/// The process override is for captures/tests and never changes preferences.
pub fn select(preference: &str, process_override: Option<&str>) -> bool {
    select_with(preference, false, process_override)
}

/// `select`, with the user's catalog file instead of `preference` when
/// `user` and the file is installed. A process override still wins, so
/// captures and tests keep their language.
pub fn select_with(preference: &str, user: bool, process_override: Option<&str>) -> bool {
    let next = if user && process_override.is_none() && USER.get().is_some() {
        USER_INDEX
    } else {
        let requested = process_override.unwrap_or(preference);
        let code = if requested.is_empty() || requested == "default" {
            system_language()
        } else {
            requested
        };
        index(code).unwrap_or_else(english)
    };
    CURRENT.swap(next, Ordering::AcqRel) != next
}

fn current() -> &'static Catalog {
    let mut selected = CURRENT.load(Ordering::Acquire);
    if selected == usize::MAX {
        // the system's language, unless a selection came in the meantime
        let system = index(system_language()).unwrap_or_else(english);
        let _ = CURRENT.compare_exchange(usize::MAX, system, Ordering::AcqRel, Ordering::Acquire);
        selected = CURRENT.load(Ordering::Acquire);
    }
    match (selected, USER.get()) {
        (USER_INDEX, Some(user)) => user,
        (USER_INDEX, None) => bundled(english()),
        _ => bundled(selected),
    }
}

/// The catalog of an explicit language: the user's when it is the one in
/// use and has that language, else the bundled one (then English). English
/// is always the bundled one: the code asks for it by name for the
/// reference texts.
fn catalog_for(language: &str) -> &'static Catalog {
    if language != "en"
        && CURRENT.load(Ordering::Acquire) == USER_INDEX
        && let Some(user) = USER.get()
        && user.code == language
    {
        return user;
    }
    bundled(index(language).unwrap_or_else(english))
}

pub fn language() -> &'static str {
    current().code
}

pub fn available_languages() -> impl Iterator<Item = (&'static str, &'static str)> {
    SOURCES.iter().map(|&(code, source)| {
        let name = source
            .lines()
            .find_map(|line| line.strip_prefix("language-name = "))
            .map(str::trim)
            .unwrap_or(code);
        (code, name)
    })
}

/// The system's closest bundled language first, then commonly requested
/// languages and the remaining entries in language-code order.
pub fn language_menu_order(items: &mut [(&str, &str)], system: &str) {
    const COMMON: &[&str] = &[
        "en", "zh-Hans", "zh-Hant", "es", "pt-BR", "pt", "ja", "fr", "de", "ko", "ru", "hi", "ar",
    ];
    let system_code = language_index(system, items.iter().map(|(code, _)| *code)).map(|index| items[index].0);
    items.sort_unstable_by_key(|&(code, _)| {
        let rank = COMMON.iter().position(|&item| item == code).unwrap_or(COMMON.len());
        (Some(code) != system_code, rank, code)
    });
}

pub fn text(key: &'static str) -> &'static str {
    wrap(plain(current(), key))
}

/// Tea::setCurrency / setRegionCurrency: the symbol shown in place of "L$"
/// from now on; empty for "L$".
pub fn set_currency(symbol: &str) {
    let symbol = symbol.trim();
    let symbol = if symbol.is_empty() { LINDEN } else { symbol };
    if *CURRENCY.read().unwrap_or_else(|e| e.into_inner()) == symbol {
        return;
    }
    let interned = {
        let mut symbols = SYMBOLS.write().unwrap_or_else(|e| e.into_inner());
        match symbols.iter().find(|known| **known == symbol) {
            Some(known) => *known,
            None => {
                let leaked: &'static str = Box::leak(symbol.to_owned().into_boxed_str());
                symbols.push(leaked);
                leaked
            }
        }
    };
    *CURRENCY.write().unwrap_or_else(|e| e.into_inner()) = interned;
    CUSTOM.store(interned != LINDEN, Ordering::Release);
}

/// Tea::getCurrency: the symbol in use, "L$" unless a grid names its own.
pub fn currency() -> &'static str {
    if CUSTOM.load(Ordering::Acquire) {
        *CURRENCY.read().unwrap_or_else(|e| e.into_inner())
    } else {
        LINDEN
    }
}

/// Tea::wrapCurrency for a catalog's plain message.
fn wrap(text: &'static str) -> &'static str {
    if !CUSTOM.load(Ordering::Acquire) || !text.contains(LINDEN) {
        return text;
    }
    let symbol = currency();
    let key = (text.as_ptr() as usize, symbol);
    if let Some(wrapped) = WRAPPED.read().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return wrapped;
    }
    let wrapped: &'static str = Box::leak(text.replace(LINDEN, symbol).into_boxed_str());
    WRAPPED.write().unwrap_or_else(|e| e.into_inner()).insert(key, wrapped);
    wrapped
}

/// Tea::wrapCurrency for a formatted message.
fn wrap_owned(text: String) -> String {
    if CUSTOM.load(Ordering::Acquire) && text.contains(LINDEN) {
        text.replace(LINDEN, currency())
    } else {
        text
    }
}

fn plain(catalog: &'static Catalog, key: &'static str) -> &'static str {
    catalog
        .plain
        .get(key)
        .or_else(|| bundled(english()).plain.get(key))
        .map_or(key, String::as_str)
}

/// Resolve a message in an explicit locale, useful for deterministic tests.
pub fn text_in(language: &str, key: &'static str) -> &'static str {
    wrap(plain(catalog_for(language), key))
}

fn render(catalog: &Catalog, key: &str, args: &FluentArgs<'_>) -> Option<String> {
    let pattern = catalog.bundle.get_message(key)?.value()?;
    let mut errors = Vec::new();
    let result = catalog.bundle.format_pattern(pattern, Some(args), &mut errors);
    errors.is_empty().then(|| result.into_owned())
}

pub fn format(key: &'static str, args: &FluentArgs<'_>) -> String {
    wrap_owned(format_catalog(current(), key, args))
}

pub fn format_in(language: &str, key: &'static str, args: &FluentArgs<'_>) -> String {
    wrap_owned(format_catalog(catalog_for(language), key, args))
}

fn format_catalog(catalog: &Catalog, key: &'static str, args: &FluentArgs<'_>) -> String {
    render_with_fallback(catalog, bundled(english()), key, args).unwrap_or_else(|| {
        log::warn!("Missing localization message or arguments: {key}");
        key.to_owned()
    })
}

fn render_with_fallback(catalog: &Catalog, fallback: &Catalog, key: &str, args: &FluentArgs<'_>) -> Option<String> {
    render(catalog, key, args).or_else(|| render(fallback, key, args))
}

/// Display a preformatted numeric value using the catalog's decimal separator.
/// Call only for numbers: resident names, identifiers and protocol data stay intact.
pub fn number_text(value: &str) -> String {
    // the catalog in use, whatever its code: the user's file has its own
    value.replace('.', text("number-decimal-separator"))
}

#[cfg(test)]
fn number_text_in(language: &str, value: &str) -> String {
    value.replace('.', text_in(language, "number-decimal-separator"))
}

pub fn number(value: f64, precision: usize) -> String {
    number_text(&format!("{value:.precision$}"))
}

/// Group displayed integers, including prices and avatar complexity.
pub fn integer(value: i64) -> String {
    grouped(value, text("number-thousands-separator"))
}

#[cfg(test)]
fn integer_in(language: &str, value: i64) -> String {
    grouped(value, text_in(language, "number-thousands-separator"))
}

fn grouped(value: i64, separator: &str) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut result = String::new();
    if value < 0 {
        result.push('-');
    }
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            result.push_str(separator);
        }
        result.push(digit);
    }
    result
}

/// Accept the selected decimal separator and the usual dot in editable numbers.
pub fn parse_number(value: &str) -> Option<f64> {
    value.trim().replace(text("number-decimal-separator"), ".").parse().ok()
}

#[doc(hidden)]
pub use fluent_bundle::FluentArgs as Args;

/// Plain labels borrow cached text; messages with variables return a String.
#[macro_export]
macro_rules! tr {
    ($key:literal) => { $crate::text($key) };
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {{
        let mut args = $crate::Args::new();
        $(args.set(stringify!($name), $value);)+
        $crate::format($key, &args)
    }};
}

#[macro_export]
macro_rules! tr_in {
    ($language:expr, $key:literal) => { $crate::text_in($language, $key) };
    ($language:expr, $key:literal, $($name:ident = $value:expr),+ $(,)?) => {{
        let mut args = $crate::Args::new();
        $(args.set(stringify!($name), $value);)+
        $crate::format_in($language, $key, &args)
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(code: &str, source: &str) -> Catalog {
        let mut bundle = FluentBundle::new_concurrent(vec![code.parse().unwrap()]);
        bundle.set_use_isolating(false);
        bundle.add_resource(FluentResource::try_new(source.to_owned()).unwrap()).unwrap();
        Catalog {
            code: "fixture",
            bundle,
            plain: HashMap::new(),
        }
    }

    #[test]
    fn an_opensim_grid_s_currency_symbol_replaces_l_dollars_in_messages() {
        // the only test that changes the symbol: put back before the end
        assert_eq!(currency(), "L$");
        assert_eq!(text_in("en", "currency-title"), "Buy L$");
        set_currency("KC");
        assert_eq!(currency(), "KC");
        assert_eq!(text_in("en", "currency-title"), "Buy KC");
        assert_eq!(text_in("fr", "currency-title").matches("KC").count(), 1);
        assert!(
            std::ptr::eq(text_in("en", "currency-title"), text_in("en", "currency-title")),
            "cached"
        );
        assert_eq!(text_in("en", "currency-you-have"), "You now have", "without L$: untouched");
        set_currency("OS$");
        assert_eq!(text_in("en", "currency-title"), "Buy OS$");
        set_currency(" ");
        assert_eq!(currency(), "L$", "empty: back to L$");
        assert_eq!(text_in("en", "currency-title"), "Buy L$");
    }

    #[test]
    fn missing_messages_and_variables_use_the_english_message() {
        let english = fixture("en", "greeting = Hello { $name }\nonly-english = English fallback");
        let french = fixture("fr", "greeting = Bonjour { $unavailable }");
        let mut args = Args::new();
        args.set("name", "Zoé");
        assert_eq!(
            render_with_fallback(&french, &english, "greeting", &args).as_deref(),
            Some("Hello Zoé")
        );
        assert_eq!(
            render_with_fallback(&french, &english, "only-english", &args).as_deref(),
            Some("English fallback")
        );
        assert!(render(&english, "unknown-key", &args).is_none());
        assert!(render(&english, "greeting", &Args::new()).is_none());
    }

    #[test]
    fn decimal_separator_follows_the_catalog() {
        assert_eq!(number_text_in("en", "-1234.50"), "-1234.50");
        assert_eq!(number_text_in("fr", "-1234.50"), "-1234,50");
        assert_eq!(number_text_in("unsupported", "1.25"), "1.25");
        assert_eq!(number_text_in("fr", "42"), "42");
        assert_eq!(integer_in("en", 1_234_567), "1,234,567");
        assert_eq!(integer_in("fr", -88_000), "-88\u{a0}000");
        assert_eq!(integer_in("en", i64::MIN), "-9,223,372,036,854,775,808");
        assert_eq!(number_text_in("ja", "-1234.50"), "-1234.50");
        assert_eq!(integer_in("ja", 1_234_567), "1,234,567");
    }

    #[test]
    fn switching_language_refreshes_plain_labels_and_cached_tables() {
        static LABELS: Localized<Vec<&str>> = Localized::new();
        select("en", None);
        assert_eq!(text("language-system"), "System language");
        let english = LABELS.get(|| vec![text("language-system")]);
        select("fr", None);
        assert_eq!(text("language-system"), "Langue du système");
        let french = LABELS.get(|| vec![text("language-system")]);
        assert_ne!(english[0], french[0]);
        select("en", None);
        assert!(std::ptr::eq(english, LABELS.get(Vec::new)));
        assert_eq!(text("unknown-key"), "unknown-key");
        select("fr", Some("en"));
        assert_eq!(language(), "en");
        select("default", None);
    }
}
