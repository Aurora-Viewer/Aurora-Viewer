use super::*;
use serde_json::json;

/// The changes of the pages given applied (the Import window ticks
/// settings; a whole page here).
fn apply(plan: &Plan, settings: &mut Settings, pages: &[&str]) -> usize {
    plan.apply_if(settings, |c| pages.contains(&c.page))
}

/// A few settings on their pages, as Preferences declares them.
fn catalog() -> Catalog {
    Catalog::new([
        ("graphics", "draw_distance".to_owned()),
        ("graphics", "vsync".to_owned()),
        ("appearance", "colors".to_owned()),
        ("privacy", "lookat.private".to_owned()),
        ("camera", "camera.fov".to_owned()),
    ])
}

/// Settings changed on a shared page, on a page never shared, and on no
/// page at all.
fn mine() -> Settings {
    let mut s = Settings::default();
    s.draw_distance = 300.0;
    s.vsync = !s.vsync;
    s.colors.chat_direct = [1, 2, 3, 255];
    s.lookat.private = !s.lookat.private;
    s
}

#[test]
fn a_code_holds_the_changed_settings_of_the_pages_chosen_only() {
    let text = export(&mine(), &catalog(), &["graphics"], "My look");
    assert!(text.starts_with(HEADER));
    assert!(text.contains("name: My look\n"));
    assert!(text.contains("pages: graphics\n"));
    assert!(text.contains("draw_distance = 300.0\n"), "{text}");
    assert!(text.contains("vsync = "));
    assert!(!text.contains("colors."), "a page not chosen: {text}");
    // the defaults are not written
    assert!(!text.contains("camera.fov"));
}

#[test]
fn a_page_never_shared_is_never_written_even_when_asked() {
    let text = export(&mine(), &catalog(), &["graphics", "appearance", "privacy"], "");
    assert!(text.contains("colors.chat_direct = [1,2,3,255]"), "{text}");
    assert!(!text.contains("lookat"), "{text}");
    assert!(text.contains("pages: graphics, appearance\n"));
}

#[test]
fn the_login_this_pc_and_keys_never_go_in_a_code() {
    let mut s = mine();
    s.username = "someone".into();
    s.gifs.giphy_key = "a-service-key".into();
    s.cache_location = "D:\\cache".into();
    // even with every row of Preferences on a shared page
    let all = Catalog::new(
        crate::ui::options::setting_pages()
            .into_iter()
            .map(|(_, path, _)| ("graphics", path))
            .chain([("graphics", "username".to_owned()), ("graphics", "gifs".to_owned())]),
    );
    let text = export(&s, &all, &["graphics"], "");
    for value in ["someone", "a-service-key", "cache"] {
        assert!(!text.contains(value), "{value} in {text}");
    }
}

#[test]
fn a_name_is_one_short_line() {
    let text = export(&mine(), &catalog(), &["graphics"], "Line\nbreak\u{7}");
    assert!(
        text.contains("name: Line break \n") || text.contains("name: Line break\n"),
        "{text}"
    );
    let long = "x".repeat(500);
    let code = Code::parse(&export(&mine(), &catalog(), &["graphics"], &long)).unwrap();
    assert_eq!(code.name.chars().count(), MAX_NAME);
}

#[test]
fn a_code_reads_back_as_it_was_written() {
    let text = export(&mine(), &catalog(), &["graphics", "appearance"], "Evening");
    let code = Code::parse(&text).unwrap();
    assert_eq!(code.name, "Evening");
    assert_eq!(code.pages, vec!["graphics", "appearance"]);
    assert_eq!(code.skipped, 0);
    let plan = Plan::new(&code, &Settings::default(), &catalog(), &Locks::default(), false);
    assert_eq!(plan.refused, 0);
    assert_eq!(plan.changes.len(), 3, "{plan:?}");
    let mut theirs = Settings::default();
    assert_eq!(apply(&plan, &mut theirs, &["graphics", "appearance"]), 3);
    assert_eq!(theirs.draw_distance, 300.0);
    assert_eq!(theirs.vsync, mine().vsync);
    assert_eq!(theirs.colors.chat_direct, [1, 2, 3, 255]);
    // the page never shared stays the receiver's
    assert_eq!(theirs.lookat.private, Settings::default().lookat.private);
}

#[test]
fn only_the_pages_ticked_are_applied() {
    let code = Code::parse(&export(&mine(), &catalog(), &["graphics", "appearance"], "")).unwrap();
    let plan = Plan::new(&code, &Settings::default(), &catalog(), &Locks::default(), false);
    let mut theirs = Settings::default();
    assert_eq!(apply(&plan, &mut theirs, &["appearance"]), 1);
    assert_eq!(theirs.draw_distance, Settings::default().draw_distance);
    assert_eq!(theirs.colors.chat_direct, [1, 2, 3, 255]);
}

#[test]
fn what_the_receiver_already_has_is_no_change() {
    let code = Code::parse(&export(&mine(), &catalog(), &["graphics"], "")).unwrap();
    let plan = Plan::new(&code, &mine(), &catalog(), &Locks::default(), false);
    assert!(plan.changes.is_empty());
    assert_eq!(plan.same, 2);
}

#[test]
fn a_hand_written_code_cannot_reach_a_guarded_page_or_this_pc() {
    let text = format!(
        "{HEADER}\npages: graphics, privacy, rlva\nlookat.private = true\nrlva_enabled = true\nusername = \"x\"\ncache_location = \"C:\\\\x\"\nnot.a.setting = 1\ndraw_distance = 200.0\n"
    );
    let code = Code::parse(&text).unwrap();
    assert_eq!(code.pages, vec!["graphics"], "only shared pages are read");
    let plan = Plan::new(&code, &Settings::default(), &catalog(), &Locks::default(), false);
    assert_eq!(plan.changes.len(), 1, "{plan:?}");
    assert_eq!(plan.changes[0].key, "draw_distance");
    assert_eq!(plan.refused, 5);
}

#[test]
fn ids_addresses_paths_and_wrong_types_are_refused() {
    let catalog = Catalog::new([("appearance", "font".to_owned()), ("graphics", "draw_distance".to_owned())]);
    for value in [
        json!("5748decc-f629-461c-9a36-a35a221fe21f"),
        json!("https://example.org/x"),
        json!("C:\\Users\\someone\\font.ttf"),
        json!("/home/someone/font.ttf"),
        json!("someone@example.org"),
        json!("x".repeat(500)),
    ] {
        let text = format!("{HEADER}\nfont = {value}\n");
        let plan = Plan::new(
            &Code::parse(&text).unwrap(),
            &Settings::default(),
            &catalog,
            &Locks::default(),
            false,
        );
        assert!(plan.changes.is_empty(), "{value} accepted");
        assert_eq!(plan.refused, 1);
    }
    let text = format!("{HEADER}\ndraw_distance = \"far\"\n");
    let plan = Plan::new(
        &Code::parse(&text).unwrap(),
        &Settings::default(),
        &catalog,
        &Locks::default(),
        false,
    );
    assert_eq!((plan.changes.len(), plan.refused), (0, 1));
}

#[test]
fn values_out_of_bounds_take_the_settings_own_bounds() {
    let text = format!("{HEADER}\ndraw_distance = 999999.0\n");
    let plan = Plan::new(
        &Code::parse(&text).unwrap(),
        &Settings::default(),
        &catalog(),
        &Locks::default(),
        false,
    );
    let mut theirs = Settings::default();
    apply(&plan, &mut theirs, &["graphics"]);
    assert!(theirs.draw_distance < 999999.0, "{}", theirs.draw_distance);
    assert_eq!(theirs.draw_distance, theirs.clone().sanitized().draw_distance);
}

#[test]
fn the_preview_shows_the_value_apply_stores() {
    let text = format!("{HEADER}\ndraw_distance = 999999.0\n");
    let code = Code::parse(&text).unwrap();
    let plan = Plan::new(&code, &Settings::default(), &catalog(), &Locks::default(), false);
    let mut theirs = Settings::default();
    apply(&plan, &mut theirs, &["graphics"]);
    assert_eq!(plan.changes.len(), 1, "{plan:?}");
    assert_eq!(plan.changes[0].to.as_f64(), Some(f64::from(theirs.draw_distance)));
    assert_ne!(plan.changes[0].to, json!(999999.0));

    // a value that clamps to what the user already has changes nothing
    let plan = Plan::new(&code, &theirs, &catalog(), &Locks::default(), false);
    assert!(plan.changes.is_empty(), "{plan:?}");
    assert_eq!((plan.same, plan.refused), (1, 0));
}

#[test]
fn invisible_format_characters_never_travel() {
    // a bidi override, a zero-width space, an isolate, a byte order mark
    for c in ['\u{202E}', '\u{200B}', '\u{2067}', '\u{FEFF}'] {
        let text = export(&mine(), &catalog(), &["graphics"], &format!("Cozy{c} evening"));
        assert!(text.contains("name: Cozy evening\n"), "{c:?}: {text}");
        let code = Code::parse(&format!("{HEADER}\nname: Cozy{c} evening\n")).unwrap();
        assert_eq!(code.name, "Cozy evening");

        let catalog = Catalog::new([("appearance", "font".to_owned())]);
        let value = json!(format!("Noto{c} Sans"));
        let code = Code::parse(&format!("{HEADER}\nfont = {value}\n")).unwrap();
        let plan = Plan::new(&code, &Settings::default(), &catalog, &Locks::default(), false);
        assert_eq!((plan.changes.len(), plan.refused), (0, 1), "{c:?}");
        let mine = Settings {
            font: format!("Noto{c} Sans"),
            ..Settings::default()
        };
        assert!(!export(&mine, &catalog, &["appearance"], "").contains("font"), "{c:?}");
    }
}

#[test]
fn a_setting_rlva_holds_is_refused() {
    let text = format!("{HEADER}\ncamera.fov = 1.2\n");
    let locks = Locks {
        camera_fov: true,
        ..Locks::default()
    };
    let plan = Plan::new(&Code::parse(&text).unwrap(), &Settings::default(), &catalog(), &locks, false);
    assert_eq!((plan.changes.len(), plan.refused), (0, 1));
}

#[test]
fn reset_others_puts_the_rest_of_the_code_pages_back_to_default() {
    // the sender shares graphics with only the draw distance changed
    let sender = Settings {
        draw_distance: 200.0,
        ..Settings::default()
    };
    let code = Code::parse(&export(&sender, &catalog(), &["graphics"], "")).unwrap();
    let receiver = mine();
    let kept = Plan::new(&code, &receiver, &catalog(), &Locks::default(), false);
    assert_eq!(kept.changes.len(), 1);
    let reset = Plan::new(&code, &receiver, &catalog(), &Locks::default(), true);
    let vsync = reset.changes.iter().find(|c| c.key == "vsync").expect("vsync reset");
    assert!(vsync.reset);
    // another page of the receiver is not the code's to reset
    assert!(reset.changes.iter().all(|c| c.page == "graphics"), "{reset:?}");
    let mut theirs = receiver.clone();
    apply(&reset, &mut theirs, &["graphics"]);
    assert_eq!(theirs.vsync, Settings::default().vsync);
    assert_eq!(theirs.draw_distance, 200.0);
    assert_eq!(theirs.colors.chat_direct, [1, 2, 3, 255]);
}

#[test]
fn a_text_without_the_header_or_too_long_is_refused() {
    assert_eq!(Code::parse("draw_distance = 1.0"), Err(Refused::NotACode));
    assert_eq!(Code::parse(&format!("{HEADER}\n{}", "x".repeat(MAX_TEXT))), Err(Refused::TooLong));
}

#[test]
fn chat_noise_before_the_header_and_broken_lines_are_skipped() {
    let text =
        format!("[12:01] Friend: here {HEADER}\nname: Hi\ndraw_distance = 200.0\nthis is not a line\nkey = not json\n../escape = 1\n");
    let code = Code::parse(&text).unwrap();
    assert_eq!(code.values, vec![("draw_distance".to_owned(), json!(200.0))]);
    assert_eq!(code.skipped, 3);
}

#[test]
fn a_key_named_twice_keeps_its_last_value() {
    let code = Code::parse(&format!("{HEADER}\ndraw_distance = 100.0\ndraw_distance = 200.0\n")).unwrap();
    assert_eq!(code.values, vec![("draw_distance".to_owned(), json!(200.0))]);
}

#[test]
fn the_lines_read_are_capped() {
    let lines: String = (0..MAX_LINES + 50).map(|i| format!("a{i} = 1\n")).collect();
    let code = Code::parse(&format!("{HEADER}\n{lines}")).unwrap();
    assert_eq!(code.values.len(), MAX_LINES);
}

#[test]
fn every_page_of_preferences_is_shared_or_never_shared() {
    for page in crate::ui::options::Page::ALL {
        let key = page.key();
        assert!(
            SHARED_PAGES.contains(&key) ^ NEVER_SHARED.contains(&key),
            "{key} must be in exactly one list"
        );
    }
    for page in LOOKS {
        assert!(SHARED_PAGES.contains(page));
    }
}

#[test]
fn every_personal_setting_is_a_setting() {
    // a renamed setting would quietly go into the codes again
    let saved = serde_json::to_value(Settings::default().to_saved()).unwrap();
    for key in PERSONAL.iter().chain(PROTECTIVE) {
        let mut at = &saved;
        for part in key.split('.') {
            at = at.get(part).unwrap_or_else(|| panic!("{key} is not a setting"));
        }
    }
}

#[test]
fn the_real_pages_round_trip_a_changed_setting_of_each_shared_page() {
    let rows = crate::ui::options::setting_pages();
    let catalog = Catalog::new(rows.iter().map(|(page, path, _)| (*page, path.clone())));
    assert!(catalog.page("draw_distance").is_some());
    // nothing of a guarded page is shared, whatever its value
    let mut s = Settings::default();
    s.lookat.private = !s.lookat.private;
    let text = export(&s, &catalog, SHARED_PAGES, "");
    assert!(!text.contains("lookat.private"), "{text}");
    s.draw_distance = 222.0;
    let code = Code::parse(&export(&s, &catalog, SHARED_PAGES, "")).unwrap();
    let plan = Plan::new(&code, &Settings::default(), &catalog, &Locks::default(), false);
    assert!(plan.changes.iter().any(|c| c.key == "draw_distance"));
    assert_eq!(plan.refused, 0);
}

#[test]
fn personal_texts_and_this_pc_hardware_are_not_shared() {
    let catalog = Catalog::new([
        ("graphics", "night_sky_place".to_owned()),
        ("graphics", "gpu_adapter".to_owned()),
        ("chat", "autoresponse".to_owned()),
        ("chat", "keywords".to_owned()),
    ]);
    let mut s = Settings {
        night_sky_place: crate::world::night_sky::SkyPlace::Tokyo,
        gpu_adapter: "My card".into(),
        ..Settings::default()
    };
    s.autoresponse.away_text = "Back soon".into();
    s.keywords.words = "myname".into();
    let text = export(&s, &catalog, SHARED_PAGES, "");
    for value in ["Tokyo", "My card", "Back soon", "myname"] {
        assert!(!text.contains(value), "{value} in {text}");
    }
    let code = format!("{HEADER}\nnight_sky_place = \"Tokyo\"\nkeywords.words = \"x\"\n");
    let plan = Plan::new(
        &Code::parse(&code).unwrap(),
        &Settings::default(),
        &catalog,
        &Locks::default(),
        false,
    );
    assert_eq!((plan.changes.len(), plan.refused), (0, 2));
}

#[test]
fn protections_are_never_shared_nor_accepted() {
    let catalog = Catalog::new([
        ("inventory", "auto_accept_inventory".to_owned()),
        ("media", "audio.media_autoplay".to_owned()),
        ("media", "media".to_owned()),
        ("sound", "audio.stream_title_announce".to_owned()),
        ("sound", "audio.stream_title_channel".to_owned()),
        ("aurora", "use_bridge".to_owned()),
        ("sound", "audio.music_autoplay".to_owned()),
        ("media", "media.show_on_others".to_owned()),
        ("chat", "link_previews.load".to_owned()),
        ("chat", "link_previews.nearby".to_owned()),
        ("chat", "script_errors_to_channel".to_owned()),
        ("chat", "script_errors_channel".to_owned()),
        ("inventory", "inventory.warn_task_move".to_owned()),
        ("inventory", "inventory.warn_scripted_task_move".to_owned()),
        ("confirmations", "confirm_teleports".to_owned()),
        ("commands", "chat_commands".to_owned()),
        ("graphics", "draw_distance".to_owned()),
    ]);
    // a user whose protections are down shares none of it
    let mut s = Settings::default();
    s.auto_accept_inventory = !s.auto_accept_inventory;
    s.audio.media_autoplay = !s.audio.media_autoplay;
    s.media.autoplay_huds = !s.media.autoplay_huds;
    s.media.allow_scripted = !s.media.allow_scripted;
    s.media.filter = !s.media.filter;
    s.audio.stream_title_announce = !s.audio.stream_title_announce;
    s.audio.stream_title_channel = 0;
    s.use_bridge = !s.use_bridge;
    s.audio.music_autoplay = !s.audio.music_autoplay;
    s.media.show_on_others = !s.media.show_on_others;
    s.link_previews.load = crate::link_preview::Load::All;
    s.link_previews.nearby = !s.link_previews.nearby;
    s.script_errors_to_channel = !s.script_errors_to_channel;
    s.script_errors_channel = 0;
    s.inventory.warn_task_move = !s.inventory.warn_task_move;
    s.inventory.warn_scripted_task_move = !s.inventory.warn_scripted_task_move;
    s.draw_distance = 222.0;
    let all = [
        "inventory",
        "media",
        "sound",
        "aurora",
        "chat",
        "confirmations",
        "commands",
        "graphics",
    ];
    let text = export(&s, &catalog, &all, "");
    for key in [
        "auto_accept_inventory",
        "media_autoplay",
        "autoplay_huds",
        "allow_scripted",
        "media.filter",
        "stream_title",
        "use_bridge",
        "music_autoplay",
        "show_on_others",
        "link_previews",
        "script_errors",
        "warn_task_move",
        "warn_scripted_task_move",
        "confirmations",
        "commands",
    ] {
        assert!(!text.contains(key), "{key} in {text}");
    }
    assert!(text.contains("draw_distance"), "{text}");

    // a hand-written code that lowers them is refused, both ways round
    let defaults = Settings::default();
    let code = format!(
        "{HEADER}\npages: inventory, media, sound, aurora, chat, confirmations, commands\n\
         auto_accept_inventory = {}\naudio.media_autoplay = {}\nmedia.allow_scripted = {}\n\
         media.filter = {}\naudio.stream_title_announce = {}\naudio.stream_title_channel = 0\n\
         use_bridge = {}\nconfirm_teleports = {}\n\
         audio.music_autoplay = {}\nmedia.show_on_others = {}\nlink_previews.load = \"All\"\n\
         link_previews.nearby = {}\nscript_errors_to_channel = {}\nscript_errors_channel = 0\n\
         inventory.warn_task_move = {}\ninventory.warn_scripted_task_move = {}\n",
        !defaults.auto_accept_inventory,
        !defaults.audio.media_autoplay,
        !defaults.media.allow_scripted,
        !defaults.media.filter,
        !defaults.audio.stream_title_announce,
        !defaults.use_bridge,
        !defaults.confirm_teleports,
        !defaults.audio.music_autoplay,
        !defaults.media.show_on_others,
        !defaults.link_previews.nearby,
        !defaults.script_errors_to_channel,
        !defaults.inventory.warn_task_move,
        !defaults.inventory.warn_scripted_task_move,
    );
    let code = Code::parse(&code).unwrap();
    assert!(
        !code.pages.iter().any(|p| *p == "confirmations" || *p == "commands"),
        "{:?}",
        code.pages
    );
    let plan = Plan::new(&code, &defaults, &catalog, &Locks::default(), false);
    assert!(
        plan.changes.is_empty(),
        "{:?}",
        plan.changes.iter().map(|c| &c.key).collect::<Vec<_>>()
    );
    assert_eq!(plan.refused, 16);
    // nor does "reset the others" put them back to their defaults
    let plan = Plan::new(&code, &s, &catalog, &Locks::default(), true);
    assert!(
        plan.changes.iter().all(|c| c.key == "draw_distance"),
        "{:?}",
        plan.changes.iter().map(|c| &c.key).collect::<Vec<_>>()
    );
}

#[test]
fn a_setting_also_on_a_page_never_shared_is_never_shared_whatever_the_order() {
    // the no-transfer confirmation is on Graphics and Confirmations, the
    // delete confirmation on Inventory and Confirmations: either order
    let forward = Catalog::new([
        ("graphics", "confirm_no_transfer_settings".to_owned()),
        ("confirmations", "confirm_no_transfer_settings".to_owned()),
        ("inventory", "inventory.confirm_delete".to_owned()),
        ("confirmations", "inventory.confirm_delete".to_owned()),
    ]);
    let backward = Catalog::new([
        ("confirmations", "confirm_no_transfer_settings".to_owned()),
        ("graphics", "confirm_no_transfer_settings".to_owned()),
        ("confirmations", "inventory.confirm_delete".to_owned()),
        ("inventory", "inventory.confirm_delete".to_owned()),
    ]);
    for catalog in [forward, backward] {
        assert_eq!(catalog.page("confirm_no_transfer_settings"), Some("confirmations"));
        assert_eq!(catalog.page("inventory.confirm_delete"), Some("confirmations"));
    }
}

#[test]
fn confirmations_shown_on_a_shared_page_are_never_shared_nor_accepted() {
    // built from the real pages, so a new row shown on a shared page and on
    // a page never shared is caught too
    let rows = crate::ui::options::setting_pages();
    let catalog = Catalog::new(rows.iter().map(|(page, path, _)| (*page, path.clone())));
    let mut s = Settings::default();
    s.confirm_no_transfer_settings = !s.confirm_no_transfer_settings;
    s.inventory.confirm_delete = !s.inventory.confirm_delete;
    s.media.filter_trust_own_worn = !s.media.filter_trust_own_worn;
    s.draw_distance = 222.0;
    let text = export(&s, &catalog, SHARED_PAGES, "");
    for key in ["confirm_no_transfer_settings", "confirm_delete", "filter_trust_own_worn"] {
        assert!(!text.contains(key), "{key} in {text}");
    }
    assert!(text.contains("draw_distance"), "{text}");
    // "Looks only" ticks Graphics: it still carries none of them
    let text = export(&s, &catalog, LOOKS, "");
    assert!(!text.contains("confirm_no_transfer_settings"), "{text}");

    let defaults = Settings::default();
    let code = format!(
        "{HEADER}\npages: graphics, inventory, media\n\
         confirm_no_transfer_settings = {}\ninventory.confirm_delete = {}\n\
         media.filter_trust_own_worn = {}\n",
        !defaults.confirm_no_transfer_settings, !defaults.inventory.confirm_delete, !defaults.media.filter_trust_own_worn,
    );
    let plan = Plan::new(&Code::parse(&code).unwrap(), &defaults, &catalog, &Locks::default(), false);
    assert!(
        plan.changes.is_empty(),
        "{:?}",
        plan.changes.iter().map(|c| &c.key).collect::<Vec<_>>()
    );
    assert_eq!(plan.refused, 3);
}

#[test]
fn a_page_named_twice_is_kept_once() {
    let code = Code::parse(&format!("{HEADER}\npages: graphics, camera, graphics\n")).unwrap();
    assert_eq!(code.pages, ["graphics", "camera"]);
}
