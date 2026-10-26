use super::dictionaries::{self, GLOSSARY};
use super::words::{Word, words};
use super::*;

fn texts<'a>(text: &'a str, found: &[Word]) -> Vec<&'a str> {
    found.iter().map(|w| &text[w.bytes.clone()]).collect()
}

#[test]
fn words_follow_the_legacy_scan() {
    let text = "Hello wrold, don't stop: it’s l'été! ok? Ça marche";
    let found = words(text);
    // "ok" is too short (checkSpelling: under 3 characters)
    assert_eq!(texts(text, &found), ["Hello", "wrold", "don't", "stop", "it’s", "l'été", "marche"]);
    // "Ça" too: 2 characters; the ranges count characters, not bytes
    let ete = &found[5];
    assert_eq!(ete.chars, 30..35);
    assert_eq!(text.chars().skip(30).take(5).collect::<String>(), "l'été");
}

#[test]
fn numbers_names_and_addresses_are_left_alone() {
    let text = "see x3 and 2nd user_name at https://example.com/wrold or www.exemple.org/abcd, mail me@exemple.org (secondlife://Aurora%20Demo/1/2/3) end";
    assert_eq!(texts(text, &words(text)), ["see", "and", "mail", "end"]);
    // emoji codes, typed or closed; a word after ": " stays a word
    let text = "hi :hea and :smile: note: text";
    assert_eq!(texts(text, &words(text)), ["and", "note", "text"]);
    // an apostrophe ends a word unless a letter follows it
    let text = "the dogs' bones 'quoted'";
    assert_eq!(texts(text, &words(text)), ["the", "dogs", "bones", "quoted"]);
}

#[test]
fn the_word_under_the_cursor_includes_its_end() {
    let marked = vec![(6..11, "wrold".to_owned())];
    assert!(words::at(&marked, 5).is_none());
    assert_eq!(words::at(&marked, 6).map(|(_, w)| w.as_str()), Some("wrold"));
    assert_eq!(words::at(&marked, 11).map(|(_, w)| w.as_str()), Some("wrold"));
    assert!(words::at(&marked, 12).is_none());
}

#[test]
fn files_decode_like_hunspell() {
    assert_eq!(dictionaries::decode(b"\xEF\xBB\xBF2\ncaf\xC3\xA9\n", None), "2\ncafé\n");
    // not UTF-8: the encoding the affix file names
    let aff = b"SET ISO8859-1\nTRY esianrtolcdugmphbyfvkwz\n";
    assert_eq!(dictionaries::decode(b"1\ncaf\xE9\n", Some(aff)), "1\ncafé\n");
    let aff = b"SET KOI8-R\n";
    assert_eq!(dictionaries::decode(b"\xC4\xCF\xCD", Some(aff)), "дом");
    let aff = b"SET microsoft-cp1251\n";
    assert_eq!(dictionaries::decode(b"\xE4\xEE\xEC", Some(aff)), "дом");
}

/// A folder of its own for a test's user files.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = crate::settings::config_dir().join("spelling-tests").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("test folder");
    dir
}

#[test]
fn the_default_dictionaries_check_and_suggest() {
    let dir = scratch("default");
    let defaults = SpellSettings::default();
    let dict = dictionaries::build(&dir, &defaults.main, &defaults.extra).expect("en_us + glossary");
    for right in [
        "hello", "Hello", "world", "optimize", "don't", "teleport", "avatar", "notecard", "Aurora", "inworld",
    ] {
        assert!(dict.check(right), "{right}");
    }
    for wrong in ["wrold", "teh", "optimise", "notcard"] {
        assert!(!dict.check(wrong), "{wrong}");
    }
    let mut found = Vec::new();
    dict.suggest("wrold", &mut found);
    assert!(found.iter().any(|s| s == "world"), "{found:?}");
}

#[test]
fn every_bundled_dictionary_reads() {
    let dir = scratch("bundled");
    let catalogue = dictionaries::catalogue(&dir);
    assert_eq!(catalogue.len(), 11);
    assert!(catalogue.iter().all(|d| !d.imported && !d.language.is_empty()));
    let samples = [
        ("en_au", "colour"),
        ("en_gb", "colour"),
        ("en_us", "color"),
        ("da_dk", "huset"),
        ("de_de", "Häuser"),
        ("es_es", "casas"),
        ("fr_fr", "maisons"),
        ("pl_pl", "domy"),
        ("pt_br", "casas"),
        ("ru_ru", "домами"),
    ];
    for (name, word) in samples {
        let info = catalogue.iter().find(|d| d.name == name).expect(name);
        assert!(info.primary, "{name}");
        let dict = dictionaries::build(&dir, name, &[]).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(dict.check(word), "{name}: {word}");
    }
    // the glossary has no affix file: an extra dictionary only
    let glossary = catalogue.iter().find(|d| d.name == GLOSSARY).expect("glossary");
    assert!(!glossary.primary);
    assert!(dictionaries::build(&dir, GLOSSARY, &[]).is_err());
}

#[test]
fn the_users_words_count() {
    let dir = scratch("custom");
    dictionaries::append_word(&dictionaries::custom_path(&dir), "Zorblax");
    dictionaries::append_word(&dictionaries::custom_path(&dir), "Zorblax");
    dictionaries::append_word(&dictionaries::custom_path(&dir), "quuxing");
    // the Legacy format: the count, then the words
    let file = std::fs::read_to_string(dictionaries::custom_path(&dir)).expect("written");
    assert_eq!(file.lines().collect::<Vec<_>>(), ["2", "Zorblax", "quuxing"]);
    let dict = dictionaries::build(&dir, "en_us", &[]).expect("en_us");
    assert!(dict.check("Zorblax") && dict.check("quuxing"));
    assert!(!dict.check("Zorblaxes"));
}

#[test]
fn an_imported_dictionary_is_listed_used_and_removed() {
    let dir = scratch("import");
    let source = scratch("import-source");
    std::fs::write(source.join("Tiny_XX.aff"), "SET UTF-8\nSFX S Y 1\nSFX S 0 s .\n").expect("aff");
    std::fs::write(source.join("Tiny_XX.dic"), "2\nzorg/S\nblip\n").expect("dic");
    let info = dictionaries::import(&dir, &source.join("Tiny_XX.dic")).expect("imported");
    assert_eq!(
        (info.name.as_str(), info.language.as_str(), info.primary),
        ("tiny_xx", "Tiny_XX", true)
    );
    let listed = dictionaries::catalogue(&dir);
    assert!(listed.iter().any(|d| d.name == "tiny_xx" && d.imported && d.primary));
    let dict = dictionaries::build(&dir, "tiny_xx", &[]).expect("built");
    assert!(dict.check("zorgs") && dict.check("blip") && !dict.check("blips"));
    // a word list alone is an extra dictionary
    std::fs::write(source.join("words.dic"), "1\nflorp\n").expect("dic");
    let extra = dictionaries::import(&dir, &source.join("words.dic")).expect("imported");
    assert!(!extra.primary);
    let dict = dictionaries::build(&dir, "en_us", &["words".to_owned()]).expect("built");
    assert!(dict.check("florp"));
    dictionaries::remove(&dir, "tiny_xx").expect("removed");
    assert!(!dir.join("tiny_xx.dic").exists() && !dir.join("tiny_xx.aff").exists());
    assert!(!dictionaries::catalogue(&dir).iter().any(|d| d.name == "tiny_xx"));
    // only an imported one can be removed
    assert!(dictionaries::remove(&dir, "en_us").is_err());
    assert!(dictionaries::remove(&dir, "../evil").is_err());
}

#[test]
fn a_broken_import_is_refused() {
    let dir = scratch("broken");
    let source = scratch("broken-source");
    // an affix file that does not parse: no dictionary at all
    std::fs::write(source.join("bad.aff"), "FLAG nonsense\n").expect("aff");
    std::fs::write(source.join("bad.dic"), "1\nword\n").expect("dic");
    assert!(dictionaries::import(&dir, &source.join("bad.dic")).is_err());
    assert!(dictionaries::catalogue(&dir).iter().all(|d| d.name != "bad"));
}

#[test]
fn a_bad_word_line_is_left_out_as_hunspell_does() {
    let aff = "FLAG num\nSFX 10 Y 1\nSFX 10 0 s .\n";
    let dict = dictionaries::parse(aff, "3\nhouse/10\n\"A/S\"\ncar\n").expect("parsed without the bad line");
    assert!(dict.check("houses") && dict.check("car"));
}

/// The shared checker with the default dictionaries, ready.
pub(crate) fn ready() {
    configure(&egui::Context::default(), &SpellSettings::default());
    let started = std::time::Instant::now();
    while !active() {
        assert!(started.elapsed().as_secs() < 60, "the default dictionary loads: {:?}", status());
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn texts_are_checked_and_suggestions_come_from_the_thread() {
    ready();
    let found = misspelled("Hello wrold, the avatar can teleport");
    assert_eq!(found, vec![(6..11, "wrold".to_owned())]);
    let started = std::time::Instant::now();
    let suggestions = loop {
        if let Some(found) = suggestions("wrold") {
            break found;
        }
        assert!(started.elapsed().as_secs() < 30, "suggestions arrive");
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    assert!(suggestions.contains(&"world".to_owned()), "{suggestions:?}");
    assert!(suggestions.len() <= MAX_SUGGESTIONS);
}
