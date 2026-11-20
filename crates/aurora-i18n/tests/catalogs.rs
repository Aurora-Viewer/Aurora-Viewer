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
fn bundled_catalogs_use_english_keys_and_variables() {
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
        assert!(messages.contains_key("language-name"), "{language}: missing language name");
        if matches!(*language, "en" | "fr" | "ja") {
            assert_eq!(messages, english, "{language}: keys or variables differ");
        } else {
            for (key, variables) in messages {
                assert_eq!(
                    Some(variables),
                    english.get(key),
                    "{language}/{key}: unknown key or different variables"
                );
            }
        }
    }
}

#[test]
fn advertised_names_match_fluent_catalogs() {
    for (code, name) in aurora_i18n::available_languages() {
        assert_eq!(name, aurora_i18n::text_in(code, "language-name"));
        assert_ne!(name, code, "{code}: expected a literal native name");
    }
}

#[test]
fn russian_is_available_and_uses_russian_plural_forms() {
    use fluent_bundle::FluentArgs;

    let languages: Vec<_> = aurora_i18n::available_languages().collect();
    assert!(languages.iter().any(|&(code, name)| code == "ru" && name == "Русский"));
    let codes: Vec<_> = languages.iter().map(|&(code, _)| code).collect();
    assert_eq!(resolve_language("default", "ru-RU", &codes), "ru");
    assert_eq!(resolve_language("en", "ru-RU", &codes), "en");

    for (count, ending) in [(1, "1 строка)"), (2, "2 строки)"), (5, "5 строк)"), (21, "21 строка)")] {
        let mut args = FluentArgs::new();
        args.set("name", "ru.ftl");
        args.set("count", count);
        let status = aurora_i18n::format_in("ru", "language-file-in-use", &args);
        assert!(status.ends_with(ending), "{status}");
    }
}

#[test]
fn german_uses_localized_dates_and_number_separators() {
    use fluent_bundle::FluentArgs;

    let codes: Vec<_> = aurora_i18n::available_languages().map(|(code, _)| code).collect();
    assert_eq!(resolve_language("default", "de-DE", &codes), "de");
    assert_eq!(aurora_i18n::text_in("de", "language-name"), "Deutsch");
    assert_eq!(aurora_i18n::text_in("de", "number-decimal-separator"), ",");
    assert_eq!(aurora_i18n::text_in("de", "number-thousands-separator"), ".");
    assert_eq!(aurora_i18n::text_in("de", "extra-sun"), "So");

    let mut args = FluentArgs::new();
    args.set("day", "16");
    args.set("month", "November");
    args.set("year", "2026");
    assert_eq!(aurora_i18n::format_in("de", "date-day-month-year", &args), "16. November 2026");
}

#[test]
fn brazilian_portuguese_uses_regional_dates_numbers_and_counts() {
    use fluent_bundle::FluentArgs;

    let codes: Vec<_> = aurora_i18n::available_languages().map(|(code, _)| code).collect();
    for locale in ["pt-BR", "pt_BR"] {
        assert_eq!(resolve_language("default", locale, &codes), "pt-BR");
    }
    assert_eq!(aurora_i18n::text_in("pt-BR", "language-name"), "Português (Brasil)");
    assert_eq!(aurora_i18n::text_in("pt-BR", "number-decimal-separator"), ",");
    assert_eq!(aurora_i18n::text_in("pt-BR", "number-thousands-separator"), ".");
    assert_eq!(aurora_i18n::text_in("pt-BR", "extra-sun"), "dom");

    let mut date = FluentArgs::new();
    date.set("day", "16");
    date.set("month", "novembro");
    date.set("year", "2026");
    assert_eq!(
        aurora_i18n::format_in("pt-BR", "date-day-month-year", &date),
        "16 de novembro de 2026"
    );

    for (count, expected) in [(0, "0 itens"), (1, "1 item"), (2, "2 itens")] {
        let mut args = FluentArgs::new();
        args.set("count", count);
        assert_eq!(aurora_i18n::format_in("pt-BR", "language-item-count", &args), expected);
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
fn system_locale_selects_the_correct_chinese_script_and_saved_choice_wins() {
    let available = ["en", "ja", "ko", "zh-Hans", "zh-Hant"];
    for locale in ["zh-CN", "zh_SG", "zh", "zh-Hans-CN"] {
        assert_eq!(resolve_language("default", locale, &available), "zh-Hans", "{locale}");
    }
    for locale in ["ZH_tw", "zh-HK", "zh-MO", "zh-Hant-HK"] {
        assert_eq!(resolve_language("default", locale, &available), "zh-Hant", "{locale}");
    }
    assert_eq!(resolve_language("ja-JP", "zh-TW", &available), "ja");
    assert_eq!(resolve_language("default", "ko-KR", &available), "ko");
    assert_eq!(resolve_language("default", "zh-TW", &["en", "zh-Hans"]), "en");
    assert_eq!(resolve_language("default", "zh-TW", &["en", "zh-TW", "zh-Hant"]), "zh-TW");
}

#[test]
fn another_region_of_the_language_comes_before_english() {
    assert_eq!(resolve_language("default", "pt-PT", &["en", "pt-BR"]), "pt-BR");
    assert_eq!(resolve_language("default", "pt", &["en", "pt-BR"]), "pt-BR");
    assert_eq!(resolve_language("default", "pt-PT", &["en", "pt-BR", "pt-PT"]), "pt-PT");
    assert_eq!(resolve_language("default", "pt-BR", &["en", "pt-BR", "pt-PT"]), "pt-BR");
    assert_eq!(resolve_language("default", "es-MX", &["en", "es"]), "es");
    assert_eq!(resolve_language("default", "nb-NO", &["en", "da"]), "en");
    assert_eq!(resolve_language("default", "no-NO", &["en", "nb"]), "nb");
    assert_eq!(resolve_language("default", "nn-NO", &["en", "nb"]), "nb");
    assert_eq!(resolve_language("default", "no", &["en", "da"]), "en");
}

#[test]
fn language_menu_prefers_system_then_common_then_code_order() {
    let mut choices = [
        ("vi", "Tiếng Việt"),
        ("en", "English"),
        ("da", "Dansk"),
        ("es", "Español"),
        ("zh-Hant", "繁體中文"),
    ];
    aurora_i18n::language_menu_order(&mut choices, "zh-TW");
    assert_eq!(choices.map(|(code, _)| code), ["zh-Hant", "en", "es", "da", "vi"]);
    aurora_i18n::language_menu_order(&mut choices, "vi-VN");
    assert_eq!(choices.map(|(code, _)| code), ["vi", "en", "zh-Hant", "es", "da"]);
    aurora_i18n::language_menu_order(&mut choices, "ko-KR");
    assert_eq!(choices.map(|(code, _)| code), ["en", "zh-Hant", "es", "da", "vi"]);
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
