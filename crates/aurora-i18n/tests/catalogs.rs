use aurora_i18n::{SOURCES, resolve_language};
use fluent_syntax::ast;
use std::collections::{BTreeMap, BTreeSet};

fn inline(value: &ast::InlineExpression<&str>, variables: &mut BTreeSet<String>) {
    match value {
        ast::InlineExpression::VariableReference { id } => {
            variables.insert(id.name.to_owned());
        }
        ast::InlineExpression::FunctionReference { arguments, .. } => {
            for value in &arguments.positional {
                inline(value, variables);
            }
            for value in &arguments.named {
                inline(&value.value, variables);
            }
        }
        ast::InlineExpression::TermReference {
            arguments: Some(arguments),
            ..
        } => {
            for value in &arguments.positional {
                inline(value, variables);
            }
            for value in &arguments.named {
                inline(&value.value, variables);
            }
        }
        ast::InlineExpression::Placeable { expression: value } => expression(value, variables),
        _ => {}
    }
}

fn expression(value: &ast::Expression<&str>, variables: &mut BTreeSet<String>) {
    match value {
        ast::Expression::Inline(value) => inline(value, variables),
        ast::Expression::Select { selector, variants } => {
            inline(selector, variables);
            for variant in variants {
                pattern(&variant.value, variables);
            }
        }
    }
}

fn pattern(value: &ast::Pattern<&str>, variables: &mut BTreeSet<String>) {
    for element in &value.elements {
        if let ast::PatternElement::Placeable { expression: value } = element {
            expression(value, variables);
        }
    }
}

#[test]
fn every_language_has_english_keys_and_variables() {
    let mut catalogs = BTreeMap::new();
    for &(language, source) in SOURCES {
        let resource = fluent_syntax::parser::parse(source).unwrap_or_else(|errors| panic!("{language}: {errors:?}"));
        let mut messages = BTreeMap::new();
        for entry in resource.body {
            if let ast::Entry::Message(message) = entry {
                let mut variables = BTreeSet::new();
                if let Some(value) = &message.value {
                    pattern(value, &mut variables);
                }
                for attribute in &message.attributes {
                    pattern(&attribute.value, &mut variables);
                }
                assert!(
                    messages.insert(message.id.name, variables).is_none(),
                    "duplicate {language} key {}",
                    message.id.name
                );
            }
        }
        catalogs.insert(language, messages);
    }
    let english = catalogs.get("en").expect("English fallback");
    assert!(english.contains_key("language-name"));
    for (language, messages) in &catalogs {
        assert_eq!(messages, english, "{language}: keys or variables differ");
    }
}

#[test]
fn saved_choice_system_locale_and_english_fallback() {
    let languages = ["en", "fr"];
    assert_eq!(resolve_language("en", "fr-FR", &languages), "en");
    assert_eq!(resolve_language("default", "fr-FR", &languages), "fr");
    assert_eq!(resolve_language("", "FR_fr", &languages), "fr");
    assert_eq!(resolve_language("fr", "en-US", &languages), "fr");
    assert_eq!(resolve_language("default", "de-DE", &languages), "en");
    assert_eq!(resolve_language("unknown", "fr", &languages), "en");
    assert_eq!(resolve_language("default", "", &languages), "en");
    assert_eq!(resolve_language("pt-BR", "fr", &["en", "pt-BR", "pt"]), "pt-BR");
    let languages = ["en", "fr", "ja"];
    assert_eq!(resolve_language("default", "ja-JP", &languages), "ja");
    assert_eq!(resolve_language("ja", "en-US", &languages), "ja");
    assert!(SOURCES.iter().any(|&(code, _)| code == "ja"), "Japanese catalog is bundled");
}

#[test]
fn fluent_renders_english_and_french_plural_rules() {
    use fluent_bundle::{FluentArgs, FluentBundle, FluentResource};
    for &(code, source) in SOURCES.iter().filter(|&&(code, _)| code == "en" || code == "fr") {
        let mut bundle = FluentBundle::new(vec![code.parse().unwrap()]);
        bundle.set_use_isolating(false);
        bundle.add_resource(FluentResource::try_new(source.to_owned()).unwrap()).unwrap();
        for count in [0, 1, 2] {
            let mut args = FluentArgs::new();
            args.set("count", count);
            let pattern = bundle.get_message("language-item-count").unwrap().value().unwrap();
            let mut errors = Vec::new();
            let actual = bundle.format_pattern(pattern, Some(&args), &mut errors);
            assert!(errors.is_empty(), "{errors:?}");
            let word = match (code, count) {
                ("en", 1) => "item",
                ("en", _) => "items",
                ("fr", 0 | 1) => "élément",
                _ => "éléments",
            };
            assert_eq!(actual, format!("{count} {word}"));
        }
    }
}

#[test]
fn short_dates_follow_the_language_order() {
    use aurora_i18n::tr_in;
    assert_eq!(tr_in!("en", "date-day-month", day = "28", month = "Oct"), "28 Oct");
    assert_eq!(
        tr_in!("ja", "date-day-month", day = "28", month = tr_in!("ja", "extra-oct")),
        "10月28日"
    );
    assert_eq!(
        tr_in!(
            "ja",
            "date-day-month-year",
            day = "6",
            month = tr_in!("ja", "extra-jun"),
            year = "2022"
        ),
        "2022年6月6日"
    );
}

#[test]
fn japanese_counts_have_a_single_form() {
    let &(_, source) = SOURCES.iter().find(|&&(code, _)| code == "ja").expect("Japanese catalog");
    let mut bundle = fluent_bundle::FluentBundle::new(vec!["ja".parse().unwrap()]);
    bundle.set_use_isolating(false);
    bundle
        .add_resource(fluent_bundle::FluentResource::try_new(source.to_owned()).unwrap())
        .unwrap();
    let forms: BTreeSet<String> = [0, 1, 2, 25]
        .into_iter()
        .map(|count| {
            let mut args = fluent_bundle::FluentArgs::new();
            args.set("count", count);
            let pattern = bundle.get_message("language-item-count").unwrap().value().unwrap();
            let mut errors = Vec::new();
            let actual = bundle.format_pattern(pattern, Some(&args), &mut errors);
            assert!(errors.is_empty(), "{errors:?}");
            assert!(actual.contains(&count.to_string()), "{actual}");
            actual.replace(&count.to_string(), "#")
        })
        .collect();
    assert_eq!(forms.len(), 1, "{forms:?}");
}

#[test]
fn every_bundled_message_formats_with_its_declared_variables() {
    use fluent_bundle::{FluentArgs, FluentBundle, FluentResource};
    for &(code, source) in SOURCES {
        let parsed = fluent_syntax::parser::parse(source).expect("Fluent syntax");
        let mut bundle = FluentBundle::new(vec![code.parse().expect("locale")]);
        bundle.set_use_isolating(false);
        bundle
            .add_resource(FluentResource::try_new(source.to_owned()).expect("catalog"))
            .expect("unique keys");
        for entry in parsed.body {
            let ast::Entry::Message(message) = entry else { continue };
            let mut variables = BTreeSet::new();
            pattern(message.value.as_ref().expect("message value"), &mut variables);
            let mut args = FluentArgs::new();
            for variable in variables {
                if variable == "count" {
                    args.set(variable, 2);
                } else {
                    args.set(variable, "Zoé");
                }
            }
            let value = bundle.get_message(message.id.name).expect("message").value().expect("pattern");
            let mut errors = Vec::new();
            let _ = bundle.format_pattern(value, Some(&args), &mut errors);
            assert!(errors.is_empty(), "{code}/{}: {errors:?}", message.id.name);
        }
    }
}

#[test]
fn japanese_system_locales_select_the_japanese_catalog() {
    let languages = ["en", "fr", "ja"];
    assert_eq!(resolve_language("default", "ja-JP", &languages), "ja");
    assert_eq!(resolve_language("", "ja_JP", &languages), "ja");
    assert_eq!(resolve_language("ja", "en-US", &languages), "ja");
    assert_eq!(resolve_language("default", "ja-JP", &["en", "fr"]), "en");
    assert!(aurora_i18n::available_languages().any(|language| language == ("ja", "日本語")));
    // Region and underscore variants resolve without touching the global language.
    assert_eq!(aurora_i18n::text_in("ja_JP", "language-system"), "システムの言語");
    assert_eq!(aurora_i18n::text_in("JA-jp", "language-choice"), "言語");
}

#[test]
fn japanese_dates_and_numbers_follow_japanese_conventions() {
    assert_eq!(
        aurora_i18n::tr_in!("ja", "profile-date", month = "09", day = "12", year = "2026"),
        "2026年09月12日"
    );
    assert_eq!(aurora_i18n::text_in("ja", "number-decimal-separator"), ".");
    assert_eq!(aurora_i18n::text_in("ja", "number-thousands-separator"), ",");
}

#[test]
fn japanese_falls_back_to_english_only_for_unknown_keys() {
    assert_ne!(
        aurora_i18n::text_in("ja", "language-system"),
        aurora_i18n::text_in("en", "language-system")
    );
    assert_eq!(aurora_i18n::text_in("ja", "unknown-key"), "unknown-key");
}

#[test]
fn brazilian_portuguese_selection_dates_numbers_and_plurals() {
    use aurora_i18n::{text_in, tr_in};
    let languages: Vec<_> = SOURCES.iter().map(|&(code, _)| code).collect();
    for locale in ["pt-BR", "pt_BR", "PT-br"] {
        assert_eq!(resolve_language("default", locale, &languages), "pt-BR");
        assert_eq!(text_in(locale, "language-system"), "Idioma do sistema");
    }
    assert!(aurora_i18n::available_languages().any(|language| language == ("pt-BR", "Português (Brasil)")));
    assert_eq!(text_in("pt-BR", "number-decimal-separator"), ",");
    assert_eq!(text_in("pt-BR", "number-thousands-separator"), ".");
    assert_eq!(
        tr_in!("pt-BR", "profile-date", month = "10", day = "30", year = "2026"),
        "30/10/2026"
    );
    for (count, expected) in [(0, "0 item"), (1, "1 item"), (2, "2 itens"), (25, "25 itens")] {
        assert_eq!(tr_in!("pt-BR", "language-item-count", count = count), expected);
    }
    assert_eq!(text_in("pt-BR", "unknown-key"), "unknown-key");
    // These values are serialized identities or executable commands, not labels.
    for (key, expected) in [
        ("keybinds-middle", "Middle"),
        ("keybinds-back", "Back"),
        ("keybinds-forward", "Forward"),
        ("rlva-tools-input-hint", "@version=0"),
    ] {
        assert_eq!(text_in("pt-BR", key), expected);
    }
}
