use super::*;

/// A scratch folder, removed with the test.
struct Folder(PathBuf);
impl Folder {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("aurora-script-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("scratch folder");
        Self(root)
    }
}
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const SCRIPT: &str = "default\n{\n    state_entry()\n    {\n        llSay(0, \"Hello\");\n    }\n}\n";

#[test]
fn a_byte_order_mark_is_removed() {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(SCRIPT.as_bytes());
    assert_eq!(decode(&bytes).as_deref(), Ok(SCRIPT));
    // only a leading mark: the same character later in the text is the author's
    let inner = format!("// {}\n", '\u{feff}');
    assert_eq!(decode(inner.as_bytes()).as_deref(), Ok(inner.as_str()));
}

#[test]
fn windows_and_old_mac_line_ends_become_line_feeds() {
    assert_eq!(decode(b"a\r\nb\rc\nd\r\n\r\ne").as_deref(), Ok("a\nb\nc\nd\n\ne"));
    assert_eq!(decode(b"x\r").as_deref(), Ok("x\n"));
    // a carriage return before a line feed is one line end, not two
    assert_eq!(normalize_newlines("\r\n"), "\n");
    assert_eq!(normalize_newlines("\r\r\n"), "\n\n");
    assert_eq!(normalize_newlines("no line end"), "no line end");
}

#[test]
fn text_that_is_not_utf8_is_refused() {
    assert_eq!(decode(&[b'a', 0xFF, b'b']), Err(Problem::NotUtf8));
    // Latin-1 "é"
    assert_eq!(decode(b"// caf\xE9\n"), Err(Problem::NotUtf8));
    // UTF-16 with its mark
    assert_eq!(decode(&[0xFF, 0xFE, b'a', 0]), Err(Problem::NotUtf8));
    assert_eq!(decode("// café, 日本語\n".as_bytes()).as_deref(), Ok("// café, 日本語\n"));
}

#[test]
fn an_empty_or_blank_file_is_refused() {
    assert_eq!(decode(b""), Err(Problem::Empty));
    assert_eq!(decode(&[0xEF, 0xBB, 0xBF]), Err(Problem::Empty), "a mark alone");
    assert_eq!(decode(b" \r\n\t\n"), Err(Problem::Empty));
    assert!(decode(b"//\n").is_ok(), "a comment is a script");
}

#[test]
fn the_size_limit_is_the_grids_and_ignores_the_mark() {
    let at_limit = vec![b'a'; MAX_SOURCE];
    assert_eq!(decode(&at_limit).map(|text| text.len()), Ok(MAX_SOURCE));
    let over = vec![b'a'; MAX_SOURCE + 1];
    assert_eq!(decode(&over), Err(Problem::TooLarge));
    let mut marked = vec![0xEF, 0xBB, 0xBF];
    marked.extend_from_slice(&at_limit);
    assert!(decode(&marked).is_ok(), "the mark does not count");
    // too large is told before the text is read: a binary file that large is not "not UTF-8"
    assert_eq!(decode(&vec![0xFF; MAX_SOURCE + 1]), Err(Problem::TooLarge));
}

#[test]
fn problems_have_a_message_each() {
    let messages: HashSet<_> = [Problem::Unreadable, Problem::NotUtf8, Problem::Empty, Problem::TooLarge]
        .into_iter()
        .map(Problem::message)
        .collect();
    assert_eq!(messages.len(), 4);
}

#[test]
fn only_lsl_files_are_scripts_in_any_case() {
    for name in ["a.lsl", "a.LSL", "a.Lsl", "dir.v2/a.lsl", "a.b.lsl"] {
        assert!(is_script(Path::new(name)), "{name}");
    }
    for name in ["a.lsl.txt", "a.lslx", "lsl", "a.", "a.slua", "a.lua", ".lsl.bak"] {
        assert!(!is_script(Path::new(name)), "{name}");
    }
}

#[test]
fn a_file_is_read_and_normalised() {
    let dir = Folder::new();
    let path = dir.0.join("Door.LSL");
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(SCRIPT.replace('\n', "\r\n").as_bytes());
    std::fs::write(&path, bytes).expect("script file");
    let draft = read(&path).expect("script");
    assert_eq!(&*draft.text, SCRIPT);
    assert_eq!(draft.lines, 7);
    assert_eq!(draft.bytes(), SCRIPT.len());

    assert_eq!(read(&dir.0.join("missing.lsl")), Err(Problem::Unreadable));
    std::fs::write(dir.0.join("empty.lsl"), b"").expect("empty file");
    assert_eq!(read(&dir.0.join("empty.lsl")), Err(Problem::Empty));
    std::fs::write(dir.0.join("big.lsl"), vec![b'a'; MAX_SOURCE + 1]).expect("large file");
    assert_eq!(read(&dir.0.join("big.lsl")), Err(Problem::TooLarge));
    std::fs::write(dir.0.join("latin.lsl"), b"// caf\xE9").expect("latin file");
    assert_eq!(read(&dir.0.join("latin.lsl")), Err(Problem::NotUtf8));
}

#[test]
fn lines_are_counted_like_an_editor_shows_them() {
    assert_eq!(line_count("a"), 1);
    assert_eq!(line_count("a\n"), 1);
    assert_eq!(line_count("a\nb"), 2);
    assert_eq!(line_count("a\n\nb\n"), 3);
    assert_eq!(line_count(SCRIPT), 7);
}

#[test]
fn the_head_stops_at_a_line_end_and_says_when_more_follows() {
    assert_eq!(head("a\nb\nc\n", 2), ("a\nb", true));
    assert_eq!(head("a\nb\nc\n", 3), ("a\nb\nc", false));
    assert_eq!(head("a\nb\nc", 3), ("a\nb\nc", false));
    assert_eq!(head("a\nb\nc", 10), ("a\nb\nc", false));
    assert_eq!(head("a\nb", 0), ("", true));
    assert_eq!(head("", 5), ("", false));
    let long = "x\n".repeat(PREVIEW_LINES + 5);
    let draft = Draft::new(long);
    let (shown, more) = draft.head();
    assert!(more);
    assert_eq!(shown.lines().count(), PREVIEW_LINES);
}

#[test]
fn item_names_are_the_file_stem_cleaned_to_the_inventory_limit() {
    assert_eq!(item_name(Path::new("C:/scripts/Door opener.lsl")), "Door opener");
    assert_eq!(item_name(Path::new("a.b.c.LSL")), "a.b.c");
    assert_eq!(item_name(Path::new("  spaced  .lsl")), "spaced");
    // the inventory forbids '|' and non-ASCII names, as for every upload
    assert_eq!(item_name(Path::new("a|b.lsl")), "a?b");
    let long = "n".repeat(100);
    assert_eq!(item_name(Path::new(&format!("{long}.lsl"))).len(), 63);
    // nothing usable left: the default, never an empty name the grid refuses
    assert_eq!(item_name(Path::new("   .lsl")), "New Script");
    assert_eq!(item_name(Path::new("\u{7f}\u{7f}.lsl")), "New Script");
}

#[test]
fn errors_are_kept_for_the_request_they_answer_only() {
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    let mut pending = Pending::default();
    assert!(pending.take(first).is_empty());
    pending.errors(first, vec!["(1, 1) ERROR".into()]);
    assert_eq!(pending.take(second), Vec::<String>::new(), "another request does not take them");
    pending.errors(first, vec!["(1, 1) ERROR".into()]);
    assert_eq!(pending.take(first), vec!["(1, 1) ERROR".to_owned()]);
    assert!(pending.take(first).is_empty(), "taken once");
}
