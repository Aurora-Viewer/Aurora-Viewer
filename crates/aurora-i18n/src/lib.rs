//! Shared UI language and Fluent messages, including network notifications.
//! Selection follows LLUI::getUILanguage (indra/llui/llui.cpp): saved choice,
//! system language, then English. Aurora has no separate installer language.

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

pub use fluent_bundle::FluentValue;
include!(concat!(env!("OUT_DIR"), "/locales.rs"));

struct Catalog {
    code: &'static str,
    bundle: FluentBundle<FluentResource>,
    // Resolve plain messages once, so labels neither allocate nor lock in
    // the frame loop. Both catalogs live for the lifetime of the process.
    plain: HashMap<String, String>,
}

static CATALOGS: OnceLock<Vec<Catalog>> = OnceLock::new();
static CURRENT: AtomicUsize = AtomicUsize::new(usize::MAX);
static SYSTEM: OnceLock<String> = OnceLock::new();

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
        let catalog = current();
        let index = catalogs()
            .iter()
            .position(|item| item.code == catalog.code)
            .expect("current catalog is bundled");
        self.0.get_or_init(|| catalogs().iter().map(|_| OnceLock::new()).collect())[index].get_or_init(build)
    }
}

fn catalogs() -> &'static [Catalog] {
    CATALOGS.get_or_init(|| {
        SOURCES
            .iter()
            .map(|&(code, source)| {
                let locale = code.parse().expect("bundled locale filename is a language identifier");
                let resource = FluentResource::try_new(source.to_owned()).expect("bundled Fluent syntax is checked by tests");
                let keys: Vec<_> = resource
                    .entries()
                    .filter_map(|entry| match entry {
                        fluent_syntax::ast::Entry::Message(message) => Some(message.id.name.to_owned()),
                        _ => None,
                    })
                    .collect();
                let mut bundle = FluentBundle::new_concurrent(vec![locale]);
                bundle.set_use_isolating(false);
                bundle.add_resource(resource).expect("bundled keys are unique");
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
                Catalog { code, bundle, plain }
            })
            .collect()
    })
}

fn english() -> usize {
    catalogs()
        .iter()
        .position(|catalog| catalog.code == "en")
        .expect("English fallback catalog is bundled")
}

fn index(code: &str) -> Option<usize> {
    let normalized = if code.contains('_') {
        std::borrow::Cow::Owned(code.replace('_', "-"))
    } else {
        std::borrow::Cow::Borrowed(code)
    };
    catalogs()
        .iter()
        .position(|catalog| catalog.code.eq_ignore_ascii_case(&normalized))
        .or_else(|| {
            let base = normalized.split('-').next()?;
            catalogs().iter().position(|catalog| catalog.code.eq_ignore_ascii_case(base))
        })
}

/// Pure selection, independent from the machine's locale and saved files.
pub fn resolve_language<'a>(preference: &str, system: &str, available: &'a [&str]) -> &'a str {
    let requested = if preference.is_empty() || preference == "default" {
        system
    } else {
        preference
    };
    let normalized = requested.replace('_', "-");
    available
        .iter()
        .find(|code| code.eq_ignore_ascii_case(&normalized))
        .or_else(|| {
            let base = normalized.split('-').next().unwrap_or_default();
            available.iter().find(|code| code.eq_ignore_ascii_case(base))
        })
        .copied()
        .unwrap_or("en")
}

pub fn system_language() -> &'static str {
    SYSTEM.get_or_init(|| sys_locale::get_locale().unwrap_or_else(|| "en".to_owned()))
}

/// The process override is for captures/tests and never changes preferences.
pub fn select(preference: &str, process_override: Option<&str>) -> bool {
    let requested = process_override.unwrap_or(preference);
    let code = if requested.is_empty() || requested == "default" {
        system_language()
    } else {
        requested
    };
    let next = index(code).unwrap_or_else(english);
    CURRENT.swap(next, Ordering::AcqRel) != next
}

fn current() -> &'static Catalog {
    let mut index = CURRENT.load(Ordering::Acquire);
    if index == usize::MAX {
        select("default", None);
        index = CURRENT.load(Ordering::Acquire);
    }
    &catalogs()[index]
}

pub fn language() -> &'static str {
    current().code
}

pub fn available_languages() -> impl Iterator<Item = (&'static str, &'static str)> {
    catalogs().iter().map(|catalog| {
        let name = catalog.plain.get("language-name").map_or(catalog.code, String::as_str);
        (catalog.code, name)
    })
}

pub fn text(key: &'static str) -> &'static str {
    plain(current(), key)
}

fn plain(catalog: &'static Catalog, key: &'static str) -> &'static str {
    catalog
        .plain
        .get(key)
        .or_else(|| catalogs()[english()].plain.get(key))
        .map_or(key, String::as_str)
}

/// Resolve a message in an explicit locale, useful for deterministic tests.
pub fn text_in(language: &str, key: &'static str) -> &'static str {
    plain(&catalogs()[index(language).unwrap_or_else(english)], key)
}

fn render(catalog: &Catalog, key: &str, args: &FluentArgs<'_>) -> Option<String> {
    let pattern = catalog.bundle.get_message(key)?.value()?;
    let mut errors = Vec::new();
    let result = catalog.bundle.format_pattern(pattern, Some(args), &mut errors);
    errors.is_empty().then(|| result.into_owned())
}

pub fn format(key: &'static str, args: &FluentArgs<'_>) -> String {
    format_catalog(current(), key, args)
}

pub fn format_in(language: &str, key: &'static str, args: &FluentArgs<'_>) -> String {
    format_catalog(&catalogs()[index(language).unwrap_or_else(english)], key, args)
}

fn format_catalog(catalog: &Catalog, key: &'static str, args: &FluentArgs<'_>) -> String {
    render_with_fallback(catalog, &catalogs()[english()], key, args).unwrap_or_else(|| {
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
    number_text_in(language(), value)
}

fn number_text_in(language: &str, value: &str) -> String {
    value.replace('.', text_in(language, "number-decimal-separator"))
}

pub fn number(value: f64, precision: usize) -> String {
    number_text(&format!("{value:.precision$}"))
}

/// Group displayed integers, including prices and avatar complexity.
pub fn integer(value: i64) -> String {
    integer_in(language(), value)
}

fn integer_in(language: &str, value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let separator = text_in(language, "number-thousands-separator");
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
