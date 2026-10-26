use super::*;

fn list(name: &str, pairs: &[(&str, &str)]) -> List {
    List {
        name: name.to_owned(),
        replacements: pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect(),
    }
}

#[test]
fn the_bundled_lists_are_the_legacy_ones() {
    let lists = bundled();
    let names: Vec<&str> = lists.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Abbreviations", "Spelling Corrections"]);
    assert_eq!(replacement(&lists, "brb"), Some("be right back"));
    assert_eq!(replacement(&lists, "Britian"), Some("Britain"));
    assert!(lists[1].replacements.len() > 1000);
    // case counts
    assert_eq!(replacement(&lists, "BRB"), None);
}

#[test]
fn a_space_after_a_keyword_replaces_it() {
    let lists = vec![list("A", &[("brb", "be right back"), ("(c)", "©"), ("e.g.", "for example")])];
    assert_eq!(after_space(&lists, "brb ", 4), Some(("be right back ".to_owned(), 14)));
    assert_eq!(after_space(&lists, "ok brb ", 7), Some(("ok be right back ".to_owned(), 17)));
    // typed in the middle: the rest follows
    assert_eq!(after_space(&lists, "ok brb now", 7), Some(("ok be right back now".to_owned(), 17)));
    // FIRE-36304: punctuation inside keywords
    assert_eq!(after_space(&lists, "(c) ", 4), Some(("© ".to_owned(), 2)));
    assert_eq!(after_space(&lists, "see e.g. ", 9), Some(("see for example ".to_owned(), 16)));
    // not after a space, not a whole keyword, two spaces
    assert_eq!(after_space(&lists, "brb", 3), None);
    assert_eq!(after_space(&lists, "xbrb ", 5), None);
    assert_eq!(after_space(&lists, "brb  ", 5), None);
    assert_eq!(after_space(&lists, " ", 1), None);
    // characters, not bytes
    assert_eq!(after_space(&lists, "été brb ", 8), Some(("été be right back ".to_owned(), 18)));
}

#[test]
fn the_first_list_holding_the_word_wins() {
    let lists = vec![
        list("First", &[("lol", "laughing")]),
        list("Second", &[("lol", "lots of love"), ("ty", "thank you")]),
    ];
    assert_eq!(replacement(&lists, "lol"), Some("laughing"));
    assert_eq!(replacement(&lists, "ty"), Some("thank you"));
}

#[test]
fn lists_round_trip_in_the_legacy_format() {
    let lists = vec![
        list("Mine", &[("afk", "away"), ("q&a", "questions <and> answers")]),
        list("Empty", &[]),
    ];
    assert_eq!(parse_lists(lists_to_xml(&lists).as_bytes()).expect("parsed"), lists);
    let one = &lists[0];
    assert_eq!(&parse_exported(exported_xml(one).as_bytes()).expect("parsed"), one);
    // a Legacy export file, as LLFloaterAutoReplaceSettings writes it
    let legacy = br#"<?xml version="1.0" ?><llsd><map><key>name</key><string>Old</string><key>replacements</key><map><key>omw</key><string>on my way</string></map></map></llsd>"#;
    assert_eq!(parse_exported(legacy).expect("parsed"), list("Old", &[("omw", "on my way")]));
    // invalid lists are refused
    assert!(parse_exported(b"<llsd><map><key>name</key><string>x</string></map></llsd>").is_err());
    assert!(parse_exported(b"<llsd><array/></llsd>").is_err());
}

#[test]
fn names_and_keywords() {
    let lists = vec![list("Mine", &[]), list("Mine 2", &[])];
    assert_eq!(unique_name(&lists, "Mine"), "Mine 3");
    assert_eq!(unique_name(&lists, " Other "), "Other");
    assert_eq!(export_name("My list: 2"), "My_list__2.xml");
    assert!(valid_keyword("brb") && valid_keyword("e.g.") && valid_keyword("(c)"));
    assert!(!valid_keyword("") && !valid_keyword("two words") && !valid_keyword("a/b"));
}
