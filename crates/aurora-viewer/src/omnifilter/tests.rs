use super::*;

fn engine(rules: Vec<Rule>) -> Engine {
    let mut e = Engine::with_sets(vec![RuleSet {
        name: DEFAULT_SET.into(),
        rules,
    }]);
    e.enabled = true;
    e
}

fn on(name: &str, f: impl FnOnce(&mut Rule)) -> Rule {
    let mut r = Rule::new(name);
    r.enabled = true;
    f(&mut r);
    r
}

fn hay<'a>(source: Option<Source>, sender: &'a str, content: &'a str) -> Haystack<'a> {
    Haystack {
        source,
        sender_name: sender,
        content,
        region_name: "",
        owner_id: Uuid::nil(),
    }
}

fn name_of(e: &mut Engine, h: &Haystack<'_>) -> Option<String> {
    e.matches(h).map(|m| m.name)
}

#[test]
fn nothing_matches_while_the_filter_is_off() {
    let mut e = engine(vec![on("all", |_| {})]);
    e.enabled = false;
    assert_eq!(e.matches(&hay(Some(Source::NearbyChat), "a", "b")), None);
    assert!(e.log.is_empty());
}

#[test]
fn an_empty_enabled_rule_matches_everything_and_a_disabled_one_nothing() {
    let mut e = engine(vec![Rule::new("off"), on("all", |_| {})]);
    assert_eq!(name_of(&mut e, &hay(None, "", "")).as_deref(), Some("all"));
    assert_eq!(name_of(&mut e, &hay(Some(Source::Lure), "x", "y")).as_deref(), Some("all"));
}

#[test]
fn the_first_matching_rule_wins_and_is_logged() {
    let mut e = engine(vec![on("one", |r| r.content = "cat".into()), on("two", |r| r.content = "c".into())]);
    assert_eq!(name_of(&mut e, &hay(None, "", "a cat")).as_deref(), Some("one"));
    assert_eq!(name_of(&mut e, &hay(None, "", "dog cow")).as_deref(), Some("two"));
    assert_eq!(name_of(&mut e, &hay(None, "", "dog")), None);
    let logged: Vec<_> = e.log.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(logged, ["one", "two"]);
}

#[test]
fn substring_exact_and_case() {
    let mut e = engine(vec![on("sub", |r| {
        r.content = "Hello".into();
        r.content_case_insensitive = false;
    })]);
    assert!(e.matches(&hay(None, "", "say Hello there")).is_some());
    assert!(e.matches(&hay(None, "", "say hello there")).is_none());
    e.rules_mut()[0].content_case_insensitive = true;
    assert!(e.matches(&hay(None, "", "SAY HELLO")).is_some());
    e.rules_mut()[0].content_match = MatchType::Exact;
    assert!(e.matches(&hay(None, "", "say hello")).is_none());
    assert!(e.matches(&hay(None, "", "hELLo")).is_some());
    // Unicode case folding
    e.rules_mut()[0].content = "Élan".into();
    assert!(e.matches(&hay(None, "", "éLAN")).is_some());
}

#[test]
fn the_sender_regex_matches_the_whole_name_the_content_regex_anywhere() {
    let mut e = engine(vec![on("re", |r| {
        r.sender_name = "Spam.*".into();
        r.sender_match = MatchType::Regex;
        r.sender_case_insensitive = false;
        r.content = r"free\s+L\$".into();
        r.content_match = MatchType::Regex;
        r.content_case_insensitive = true;
    })]);
    assert!(e.matches(&hay(None, "Spam Bot", "get FREE   L$ now")).is_some());
    // ll_regex_match: the whole sender name
    assert!(e.matches(&hay(None, "A Spam Bot", "free L$")).is_none());
    // case-sensitive sender
    assert!(e.matches(&hay(None, "spam bot", "free L$")).is_none());
    assert!(e.matches(&hay(None, "Spam Bot", "no money")).is_none());
}

#[test]
fn perl_syntax_works_like_the_legacy_viewers_boost_regex() {
    // a look-ahead and a back-reference, which the regex crate alone refuses
    let mut e = engine(vec![on("perl", |r| {
        r.content = r"(\w)\1(?=!)".into();
        r.content_match = MatchType::Regex;
    })]);
    assert!(e.matches(&hay(None, "", "yess!")).is_some());
    assert!(e.matches(&hay(None, "", "yes!")).is_none());
}

#[test]
fn a_bad_regex_never_matches_and_does_not_stop_the_next_rules() {
    let mut e = engine(vec![
        on("bad", |r| {
            r.content = "([unclosed".into();
            r.content_match = MatchType::Regex;
        }),
        on("good", |r| r.content = "unclosed".into()),
    ]);
    assert_eq!(name_of(&mut e, &hay(None, "", "([unclosed")).as_deref(), Some("good"));
}

#[test]
fn a_catastrophic_regex_is_bounded() {
    let mut e = engine(vec![on("evil", |r| {
        // exponential backtracking with a back-reference forcing the
        // backtracking engine
        r.content = r"^(a+)+\1$".into();
        r.content_match = MatchType::Regex;
    })]);
    let text = format!("{}b", "a".repeat(40));
    let start = std::time::Instant::now();
    assert!(e.matches(&hay(None, "", &text)).is_none());
    assert!(
        start.elapsed() < std::time::Duration::from_secs(5),
        "bounded: {:?}",
        start.elapsed()
    );
}

#[test]
fn owner_region_and_kinds_must_all_agree() {
    let owner = Uuid::from_u128(7);
    let mut e = engine(vec![on("strict", |r| {
        r.owner_id = owner;
        r.region_name = "Bay City".into();
        r.set_source(Source::InventoryOffer, true);
        r.set_source(Source::InstantMessage, true);
    })]);
    let mut h = Haystack {
        source: Some(Source::InventoryOffer),
        sender_name: "x",
        content: "y",
        region_name: "Bay City",
        owner_id: owner,
    };
    assert!(e.matches(&h).is_some());
    h.region_name = "bay city";
    assert!(e.matches(&h).is_none(), "the region is compared exactly");
    h.region_name = "Bay City";
    h.owner_id = Uuid::from_u128(8);
    assert!(e.matches(&h).is_none());
    h.owner_id = owner;
    h.source = Some(Source::NearbyChat);
    assert!(e.matches(&h).is_none());
    // TYPES_MAX: only a rule without kinds
    h.source = None;
    assert!(e.matches(&h).is_none());
}

#[test]
fn an_unknown_match_type_never_matches() {
    let mut e = engine(vec![on("odd", |r| {
        r.content = "x".into();
        r.content_match = MatchType::Other(7);
    })]);
    assert!(e.matches(&hay(None, "", "x")).is_none());
    // an empty field still matches whatever its mode
    e.rules_mut()[0].content.clear();
    assert!(e.matches(&hay(None, "", "x")).is_some());
}

#[test]
fn a_match_carries_the_rules_answers_with_the_delay_kept_in_range() {
    let mut e = engine(vec![on("dialog", |r| {
        r.chat_replace = "hidden".into();
        r.button_reply = "Yes".into();
        r.textbox_reply = "hi".into();
        r.reply_delay = 9.0;
    })]);
    let m = e.matches(&hay(None, "", "")).expect("match");
    assert_eq!(
        (
            m.chat_replace.as_str(),
            m.button_reply.as_str(),
            m.textbox_reply.as_str(),
            m.reply_delay
        ),
        ("hidden", "Yes", "hi", 3.0)
    );
}

#[test]
fn edits_recompile() {
    let mut e = engine(vec![on("r", |r| r.content = "one".into())]);
    assert!(e.matches(&hay(None, "", "one")).is_some());
    e.rules_mut()[0].content = "two".into();
    assert!(e.matches(&hay(None, "", "one")).is_none());
    assert!(e.matches(&hay(None, "", "two")).is_some());
}

#[test]
fn rules_add_rename_move_and_remove_like_the_editor() {
    let mut e = engine(Vec::new());
    assert_eq!(e.add_rule("NEW RULE"), 0);
    // a second Add selects the rule already named so
    assert_eq!(e.add_rule("NEW RULE"), 0);
    let r = &e.current_set().rules[0];
    assert!(!r.enabled && r.sender_case_insensitive && r.content_case_insensitive);
    assert_eq!((r.sender_match, r.content_match), (MatchType::Substring, MatchType::Substring));
    assert!(e.rename_rule(0, "first"));
    assert_eq!(e.add_rule("NEW RULE"), 1);
    assert!(!e.rename_rule(1, "first"), "a taken name");
    assert!(!e.rename_rule(1, "   "), "a blank name");
    assert!(e.rename_rule(1, "second"));
    assert_eq!(e.move_rule(1, true), 0);
    assert_eq!(e.move_rule(0, true), 0, "the first stays");
    assert_eq!(e.move_rule(1, false), 1, "the last stays");
    let names: Vec<_> = e.current_set().rules.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["second", "first"]);
    e.remove_rule(0);
    assert_eq!(e.current_set().rules.len(), 1);
}

#[test]
fn rule_sets_add_clone_select_and_remove() {
    let mut e = engine(vec![on("a", |_| {})]);
    assert_eq!(e.add_set(""), Err(NameError::Blank));
    assert_eq!(e.add_set(DEFAULT_SET), Err(NameError::Duplicate));
    assert_eq!(e.add_set("Club"), Ok(1));
    assert_eq!(e.current(), 1);
    assert!(e.current_set().rules.is_empty());
    // the new set filters: nothing matches
    assert!(e.matches(&hay(None, "", "")).is_none());
    e.select(0);
    assert_eq!(e.clone_set("Copy"), Ok(2));
    assert_eq!(e.current_set().rules.len(), 1);
    assert!(e.matches(&hay(None, "", "")).is_some());
    assert!(e.remove_current_set());
    assert_eq!(e.current(), 0);
    assert!(!e.remove_current_set(), "never the Default set");
    // an index out of range falls back to the first set
    e.select(9);
    assert_eq!(e.current(), 0);
    let names: Vec<_> = e.sets().iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, [DEFAULT_SET, "Club"]);
}

#[test]
fn imported_sets_keep_their_rules_under_free_names() {
    let mut e = engine(vec![on("a", |_| {})]);
    e.add_set("Club").expect("added");
    let added = e.import(vec![
        RuleSet {
            name: DEFAULT_SET.into(),
            rules: vec![Rule::new("legacy")],
        },
        RuleSet {
            name: "Club".into(),
            rules: Vec::new(),
        },
        RuleSet {
            name: DEFAULT_SET.into(),
            rules: Vec::new(),
        },
    ]);
    assert_eq!(added, 2, "an empty Default set is not worth a copy");
    let names: Vec<_> = e.sets().iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, [DEFAULT_SET, "Club", "Default (2)", "Club (2)"]);
}

/// A version 4 file as the Legacy Viewer writes it (exportToLLSD).
const LEGACY_V4: &str = r#"<?xml version="1.0" ?>
<llsd>
<map>
  <key>Club</key>
  <map>
    <key>order</key><integer>1</integer>
    <key>rule_set</key>
    <map>
      <key>Answer</key>
      <map>
        <key>button_reply</key><string>Yes</string>
        <key>chat_replace</key><string />
        <key>content</key><string>button_name=Yes</string>
        <key>content_case_insensitive</key><boolean>1</boolean>
        <key>content_match_type</key><integer>1</integer>
        <key>enabled</key><boolean>1</boolean>
        <key>order</key><integer>0</integer>
        <key>owner_id</key><uuid>00000000-0000-0000-0000-000000000007</uuid>
        <key>region_name</key><string />
        <key>reply_delay</key><real>1.5</real>
        <key>sender_name</key><string>Greeter</string>
        <key>sender_name_case_insensitive</key><boolean>0</boolean>
        <key>sender_name_match_type</key><integer>0</integer>
        <key>textbox_reply</key><string />
        <key>types</key><array><integer>8</integer><integer>13</integer></array>
      </map>
    </map>
  </map>
  <key>Default</key>
  <map>
    <key>order</key><integer>0</integer>
    <key>rule_set</key>
    <map>
      <key>Zed</key>
      <map>
        <key>order</key><integer>0</integer>
        <key>content</key><string>zed</string>
        <key>enabled</key><boolean>1</boolean>
        <key>types</key><undef />
      </map>
      <key>Alpha</key>
      <map>
        <key>order</key><integer>1</integer>
        <key>sender_name</key><string>Alpha</string>
        <key>sender_name_match_type</key><integer>2</integer>
      </map>
    </map>
  </map>
</map>
</llsd>"#;

#[test]
fn a_legacy_version_4_file_reads_in_its_orders() {
    let sets = parse(LEGACY_V4.as_bytes()).expect("parsed");
    let names: Vec<_> = sets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, [DEFAULT_SET, "Club"]);
    let rules: Vec<_> = sets[0].rules.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(rules, ["Zed", "Alpha"], "the rules' order, not the map's");
    assert_eq!(sets[0].rules[1].sender_match, MatchType::Regex);
    // a missing field keeps the Needle's default
    assert!(!sets[0].rules[1].enabled && !sets[0].rules[1].content_case_insensitive);
    let a = &sets[1].rules[0];
    assert_eq!(a.button_reply, "Yes");
    assert_eq!(a.owner_id, Uuid::from_u128(7));
    assert_eq!(a.reply_delay, 1.5);
    assert_eq!(a.sender_match, MatchType::Exact);
    assert!(a.has_source(Source::ScriptDialog) && a.has_source(Source::UrlRequest) && !a.has_source(Source::Lure));
    assert!(a.enabled && a.content_case_insensitive && !a.sender_case_insensitive);
}

#[test]
fn a_file_written_reads_back_the_same() {
    let sets = parse(LEGACY_V4.as_bytes()).expect("parsed");
    let again = parse(to_xml(&sets).as_bytes()).expect("parsed again");
    assert_eq!(sets, again);
}

#[test]
fn an_older_file_without_rule_sets_becomes_the_default_set() {
    // versions 1 and 2: the map of rules alone, with or without orders
    let v2 = r#"<llsd><map>
      <key>b</key><map><key>order</key><integer>0</integer><key>content</key><string>bee</string><key>enabled</key><boolean>1</boolean></map>
      <key>a</key><map><key>order</key><integer>1</integer><key>types</key><array><integer>0</integer></array></map>
    </map></llsd>"#;
    let sets = parse(v2.as_bytes()).expect("parsed");
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].name, DEFAULT_SET);
    let rules: Vec<_> = sets[0].rules.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(rules, ["b", "a"]);
    assert!(sets[0].rules[1].has_source(Source::NearbyChat));
    // version 1: no order, the map's (by name)
    let v1 = r#"<llsd><map><key>z</key><map/><key>m</key><map/></map></llsd>"#;
    let rules: Vec<_> = parse(v1.as_bytes()).expect("parsed")[0]
        .rules
        .iter()
        .map(|r| r.name.clone())
        .collect();
    assert_eq!(rules, ["m", "z"]);
}

#[test]
fn a_version_2_file_with_a_rule_named_default_is_still_the_default_set() {
    // the rule "Default" has an order like any rule of version 2, but no
    // rule_set of its own: this is not a version 3 or 4 file
    let v2 = r#"<llsd><map>
      <key>Default</key><map><key>order</key><integer>0</integer><key>content</key><string>spam</string><key>enabled</key><boolean>1</boolean></map>
      <key>Other</key><map><key>order</key><integer>1</integer></map>
    </map></llsd>"#;
    let sets = parse(v2.as_bytes()).expect("parsed");
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].name, DEFAULT_SET);
    let rules: Vec<_> = sets[0].rules.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(rules, ["Default", "Other"]);
    assert_eq!(sets[0].rules[0].content, "spam");
    // a real version 3 or 4 file still reads as sets, also with an empty Default
    let v4 = r#"<llsd><map>
      <key>Default</key><map><key>order</key><integer>0</integer><key>rule_set</key><map/></map>
      <key>Club</key><map><key>order</key><integer>1</integer><key>rule_set</key><map><key>r</key><map/></map></map>
    </map></llsd>"#;
    let sets = parse(v4.as_bytes()).expect("parsed");
    let names: Vec<_> = sets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, [DEFAULT_SET, "Club"]);
}

#[test]
fn a_broken_file_is_an_error_not_a_crash() {
    assert!(parse(b"<llsd><map><key>x</key>").is_err());
    assert!(parse(b"<llsd><array/></llsd>").is_err());
    assert!(parse(b"not xml at all").is_err());
}

#[test]
fn installed_sets_put_default_first() {
    let e = Engine::with_sets(vec![RuleSet {
        name: "Club".into(),
        rules: Vec::new(),
    }]);
    let names: Vec<_> = e.sets().iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, [DEFAULT_SET, "Club"]);
}

#[test]
fn the_file_saves_after_a_pause_and_loads_on_start() {
    let dir = std::env::temp_dir().join(format!("aurora-omnifilter-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("omnifilter.xml");
    let mut e = Engine::default();
    e.load(path.clone());
    // no file yet: the Default set with its "Default" rule
    for _ in 0..100 {
        e.update(Instant::now());
        if e.loading.is_none() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(e.current_set().rules.len(), 1);
    e.add_rule("kept");
    e.update(Instant::now());
    assert!(!path.exists(), "not before the pause");
    e.save_before_exit();
    let sets = parse(&std::fs::read(&path).expect("saved")).expect("parsed");
    assert_eq!(sets[0].rules.len(), 2);
    let mut again = Engine::default();
    again.load(path.clone());
    for _ in 0..200 {
        again.update(Instant::now());
        if again.loading.is_none() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(again.has_rule("kept"));
    let _ = std::fs::remove_dir_all(&dir);
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("aurora-omnifilter-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn saves_land_in_the_order_they_were_asked() {
    let dir = temp_dir("order");
    let path = dir.join("omnifilter.xml");
    let mut e = Engine::default();
    e.load(path.clone());
    e.update(Instant::now());
    // many saves in a row, each with one more rule: the last one stays
    for n in 0..20 {
        e.add_rule(&format!("rule {n}"));
        e.save();
    }
    e.save_before_exit();
    let sets = parse(&std::fs::read(&path).expect("saved")).expect("parsed");
    assert!(sets[0].rules.iter().any(|r| r.name == "rule 19"), "the newest rules are on disk");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_write_keeps_the_rules_unsaved_and_is_tried_again_later() {
    let dir = temp_dir("retry");
    std::fs::create_dir_all(&dir).expect("dir");
    // the file's folder is a file: the write fails
    let blocker = dir.join("blocked");
    std::fs::write(&blocker, b"x").expect("blocker");
    let path = blocker.join("omnifilter.xml");
    let mut e = Engine::default();
    e.load(path.clone());
    e.update(Instant::now());
    e.add_rule("kept");
    e.save();
    for _ in 0..500 {
        e.update(Instant::now());
        if e.in_flight == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(e.in_flight, 0);
    assert!(e.dirty_since.is_some(), "not acknowledged: still to save");
    let retry_at = e.retry_after.expect("a wait before the next try");
    // the folder is fixed: nothing is written before the wait is over
    std::fs::remove_file(&blocker).expect("unblock");
    // (the rules have waited their delay: only the back-off holds the write)
    e.dirty_since = Some(retry_at - SAVE_DELAY * 2);
    e.update(retry_at - Duration::from_millis(1));
    assert_eq!(e.in_flight, 0);
    // after it, the next try writes it
    let later = retry_at + SAVE_DELAY;
    e.update(later);
    for _ in 0..500 {
        e.update(later);
        if e.in_flight == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(e.dirty_since.is_none());
    let sets = parse(&std::fs::read(&path).expect("saved")).expect("parsed");
    assert!(sets[0].rules.iter().any(|r| r.name == "kept"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_last_save_before_exit_waits_for_the_writes_under_way() {
    let dir = temp_dir("exit");
    let path = dir.join("omnifilter.xml");
    let mut e = Engine::default();
    e.load(path.clone());
    e.update(Instant::now());
    e.add_rule("late");
    e.save();
    e.add_rule("later");
    // no pause, no update: the exit alone must leave both on disk
    e.save_before_exit();
    assert_eq!(e.in_flight, 0);
    let sets = parse(&std::fs::read(&path).expect("saved")).expect("parsed");
    assert!(sets[0].rules.iter().any(|r| r.name == "later"));
    let _ = std::fs::remove_dir_all(&dir);
}
