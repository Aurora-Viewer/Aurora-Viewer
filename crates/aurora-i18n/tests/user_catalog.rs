//! The user's own catalog file (Preferences › Language and updates). Its own
//! test binary: installing it is once per process, and the other tests of
//! the crate select the bundled catalogs.

use aurora_i18n::{
    Localized, UserCatalog, UserCatalogError, install_user_catalog, language, select, select_with, text, text_in, tr, user_catalog_code,
    user_catalog_installed, user_catalog_selected,
};

#[test]
fn the_file_name_gives_the_language() {
    assert_eq!(user_catalog_code("de").as_deref(), Some("de"));
    assert_eq!(user_catalog_code("pt-BR").as_deref(), Some("pt-BR"));
    assert_eq!(user_catalog_code("pt_BR").as_deref(), Some("pt-BR"));
    assert_eq!(user_catalog_code("fil").as_deref(), Some("fil"));
    // "my-test" would be one: Burmese in a script called Test
    for name in ["", "my-translation", "custom", "und", "d3", "de-!!", "aurora de"] {
        assert_eq!(user_catalog_code(name), None, "{name:?}");
    }
}

#[test]
fn a_user_catalog_replaces_the_list_s_language_until_it_is_turned_off() {
    static LABELS: Localized<&str> = Localized::new();
    // nothing readable: not installed, and another file can come after it
    assert_eq!(install_user_catalog("de", "# only a comment\n"), Err(UserCatalogError::Empty));
    assert!(!user_catalog_installed());
    // without a file, `user` changes nothing
    select_with("fr", true, None);
    assert_eq!(text("language-system"), "Langue du système");
    let french = LABELS.get(|| text("language-system"));

    let source = "\u{feff}language-name = Deutsch\n\
                  language-system = Systemsprache\n\
                  number-decimal-separator = ,\n\
                  number-thousands-separator = .\n\
                  broken = }\n\
                  greeting = Hallo { $name }\n";
    let installed = install_user_catalog("de", source).expect("a readable file installs");
    assert_eq!(
        installed,
        UserCatalog {
            code: "de",
            named: true,
            messages: 5,
            errors: 1,
        }
    );
    assert!(user_catalog_installed());
    assert_eq!(
        install_user_catalog("es", "language-name = Español"),
        Err(UserCatalogError::AlreadyInstalled),
        "once per process: a new file applies at the next launch"
    );

    // chosen: its texts, English for what it lacks
    select_with("fr", true, None);
    assert!(user_catalog_selected());
    assert_eq!(language(), "de");
    assert_eq!(text("language-system"), "Systemsprache");
    assert_eq!(tr!("greeting", name = "Zoé"), "Hallo Zoé");
    assert_eq!(text("language-choice"), text_in("en", "language-choice"), "missing: English");
    assert_eq!(text_in("de", "language-system"), "Systemsprache");
    assert_eq!(text_in("en", "language-system"), "System language", "English stays the bundled one");
    assert_eq!(aurora_i18n::number(1.5, 1), "1,5");
    assert_eq!(aurora_i18n::integer(1_234_567), "1.234.567");
    let user = LABELS.get(|| text("language-system"));
    assert_eq!(*user, "Systemsprache", "its own slot in cached tables");

    // a command-line language wins, as for the saved choice
    select_with("fr", true, Some("ja"));
    assert_eq!(language(), "ja");
    assert!(!user_catalog_selected());

    // turned off: the list's language at once
    select_with("fr", false, None);
    assert_eq!(text("language-system"), "Langue du système");
    assert!(std::ptr::eq(french, LABELS.get(|| "rebuilt")));
    select("en", None);
    assert_eq!(text("language-system"), "System language");
}
