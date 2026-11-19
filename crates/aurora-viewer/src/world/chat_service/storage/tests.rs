//! The Chat Service's files on a temporary folder.

use super::*;
use aurora_net::chat_service::{TimeUuidKey, UUID_EPOCH, direct_conversation_id, format_time_uuid};

const AGENT: Uuid = Uuid::from_u128(0xa9e7);
const RES: Uuid = Uuid::from_u128(0x1001);

struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp() -> Dir {
    Dir(std::env::temp_dir().join(format!("aurora-chat-service-{}", Uuid::new_v4())))
}

fn key(n: u64) -> TimeUuidKey {
    TimeUuidKey {
        ticks: UUID_EPOCH + n * 10_000_000,
        tail: [i8::MIN, 0, 0, 0, 0, 0, 0, 0],
    }
}

fn row(n: u64) -> HistoryRow {
    HistoryRow {
        conversation_id: direct_conversation_id(AGENT, RES),
        msg_id: format_time_uuid(&key(n)),
        key: key(n),
        from_id: RES,
        from_name: "Some One".into(),
        message: format!("hello {n}"),
        dialog: 0,
        created_at: "2026-08-12T12:34:56Z".into(),
    }
}

fn storage(dir: &Dir) -> Storage {
    Storage::new(dir.0.clone(), AGENT)
}

#[test]
fn the_folder_is_the_accounts_own_beside_the_transcripts() {
    let account = Path::new("accounts").join("agent");
    let dir = folder(&account, "Second Life");
    assert!(dir.starts_with(&account));
    assert_eq!(dir.parent().and_then(Path::file_name), Some("chat_service".as_ref()));
    assert!(!dir.components().any(|c| c.as_os_str() == "chat_logs"));
    assert!(folder(&account, "  ").ends_with("grid"));
}

#[test]
fn only_the_names_of_the_feature_are_owned() {
    let id = "01234567-89ab-cdef-0123-456789abcdef";
    assert_eq!(canonical_archive_name(&format!("chat_service_{id}.csv")), parse_canonical_uuid(id));
    assert_eq!(canonical_archive_name(&format!("chat_service_{}.csv", id.to_uppercase())), None);
    for suffix in [".csv", ".csv.tmp", ".csv.corrupt"] {
        assert!(owned_artifact_name(&format!("chat_service_{id}{suffix}")));
    }
    for name in [INDEX_NAME, STATE_NAME, "chat_service_index.csv.tmp", "chat_service_state.xml.tmp"] {
        assert!(owned_artifact_name(name), "{name}");
    }
    let txt = format!("chat_service_{id}.txt");
    let bare = format!("chat_service_{id}");
    for name in ["chat_service_x.csv", txt.as_str(), bare.as_str(), "notes.txt"] {
        assert!(!owned_artifact_name(name), "{name}");
    }
}

#[test]
fn cutoffs_are_canonical_decimals_below_two_to_the_sixty() {
    assert_eq!(parse_decimal_ticks("0"), Some(0));
    assert_eq!(parse_decimal_ticks("123"), Some(123));
    assert_eq!(parse_decimal_ticks(&(UUID_TICK_LIMIT - 1).to_string()), Some(UUID_TICK_LIMIT - 1));
    let limit = UUID_TICK_LIMIT.to_string();
    for bad in ["", "01", "+1", "1 ", "-1", limit.as_str(), "99999999999999999999"] {
        assert_eq!(parse_decimal_ticks(bad), None, "{bad:?}");
    }
}

#[test]
fn the_state_is_safe_only_when_canonical_or_absent() {
    let dir = temp();
    let s = storage(&dir);
    let tmp = dir.0.join("chat_service_state.xml.tmp");
    assert_eq!(
        s.load_state(),
        StateLoad::Safe {
            deleted_before_ticks: 0,
            cleanup_pending: false
        }
    );

    assert!(s.write_state(42, true));
    assert_eq!(
        s.load_state(),
        StateLoad::Safe {
            deleted_before_ticks: 42,
            cleanup_pending: true
        }
    );
    let text = std::fs::read_to_string(dir.0.join(STATE_NAME)).unwrap();
    assert!(text.contains("<key>deleted_before_uuid_ticks</key>") && text.contains("<string>42</string>"));

    // a leftover temporary is discarded beside a valid state
    std::fs::write(&tmp, b"junk").unwrap();
    assert_eq!(
        s.load_state(),
        StateLoad::Safe {
            deleted_before_ticks: 42,
            cleanup_pending: true
        }
    );
    assert!(!tmp.exists());

    // a temporary alone is ambiguous
    std::fs::remove_file(dir.0.join(STATE_NAME)).unwrap();
    std::fs::write(&tmp, state_contents(7, false)).unwrap();
    assert_eq!(s.load_state(), StateLoad::Unsafe);
    std::fs::remove_file(&tmp).unwrap();

    for bad in [
        format!("{}junk", state_contents(1, false)),
        state_contents(1, false).replace("<string>1</string>", "<string>01</string>"),
        state_contents(1, false).replace("<boolean>0</boolean>", "<string>0</string>"),
        state_contents(1, false).replace("<map>", "<map><key>x</key><integer>1</integer>"),
        "not xml".to_string(),
    ] {
        std::fs::write(dir.0.join(STATE_NAME), &bad).unwrap();
        assert_eq!(s.load_state(), StateLoad::Unsafe, "{bad}");
    }
    // a folder in place of the state is never followed
    std::fs::remove_file(dir.0.join(STATE_NAME)).unwrap();
    std::fs::create_dir(dir.0.join(STATE_NAME)).unwrap();
    assert_eq!(s.load_state(), StateLoad::Unsafe);
}

#[test]
fn a_pass_is_written_whole_then_appended_while_the_file_is_unchanged() {
    let dir = temp();
    let mut s = storage(&dir);
    let fresh = s.prepare_archive(RES, 0).unwrap();
    assert_eq!(fresh.summary.state, ArchiveState::Absent);

    let first = s.publish_rows(RES, &[row(1), row(2)], false, fresh.summary).unwrap();
    assert_eq!(
        (first.state, first.row_count, first.oldest, first.newest),
        (ArchiveState::Valid, 2, Some(key(1)), Some(key(2)))
    );
    let path = dir.0.join(archive_name(RES));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with(csv::CSV_HEADER));
    assert!(!with_suffix(&path, ".tmp").exists());

    let second = s.publish_rows(RES, &[row(3)], true, first).unwrap();
    assert_eq!((second.row_count, second.oldest, second.newest), (3, Some(key(1)), Some(key(3))));
    assert_eq!(s.read_archive(RES, 0, 10).display_rows, vec![row(1), row(2), row(3)]);

    // changed behind the manager's back: refused
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"x")
        .unwrap();
    assert_eq!(s.publish_rows(RES, &[row(4)], true, second), None);
    // an expected absence that is not one
    assert_eq!(s.publish_rows(RES, &[row(4)], false, ArchiveScan::default()), None);
}

#[test]
fn an_append_starts_a_new_line_when_the_last_one_is_open() {
    let dir = temp();
    let mut s = storage(&dir);
    s.publish_rows(RES, &[row(1)], false, ArchiveScan::default()).unwrap();
    let path = dir.0.join(archive_name(RES));
    let mut bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.pop(), Some(b'\n'));
    std::fs::write(&path, &bytes).unwrap();
    let prepared = s.prepare_archive(RES, 0).unwrap();
    assert_eq!(prepared.summary.state, ArchiveState::Valid);
    s.publish_rows(RES, &[row(2)], true, prepared.summary).unwrap();
    assert_eq!(s.read_archive(RES, 0, 10).display_rows, vec![row(1), row(2)]);
}

#[test]
fn a_torn_tail_is_cut_and_a_corrupt_file_set_aside() {
    let dir = temp();
    let mut s = storage(&dir);
    s.publish_rows(RES, &[row(1), row(2)], false, ArchiveScan::default()).unwrap();
    let path = dir.0.join(archive_name(RES));
    let whole = std::fs::read(&path).unwrap();
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"\"half")
        .unwrap();

    let repaired = s.prepare_archive(RES, 0).unwrap();
    assert!(repaired.bytes_changed);
    assert_eq!((repaired.summary.state, repaired.summary.row_count), (ArchiveState::Valid, 2));
    assert_eq!(std::fs::read(&path).unwrap(), whole);

    std::fs::write(&path, b"not,the,header\n").unwrap();
    let aside = s.prepare_archive(RES, 0).unwrap();
    assert!(aside.bytes_changed);
    assert_eq!(aside.summary, ArchiveScan::default());
    assert!(!path.exists());
    assert_eq!(std::fs::read(with_suffix(&path, ".corrupt")).unwrap(), b"not,the,header\n");
    assert_eq!(s.enumerate(), (vec![], true));
}

#[test]
fn the_index_lists_the_archives_still_present() {
    let dir = temp();
    let mut s = storage(&dir);
    let other = Uuid::from_u128(0x1002);
    s.publish_rows(RES, &[row(1)], false, ArchiveScan::default()).unwrap();
    let name = crate::world::names::AvatarName {
        username: "some.one".into(),
        display_name: "Some, One".into(),
        ..Default::default()
    };
    let entries = [
        IndexEntry {
            resident: RES,
            name: Some(name),
        },
        IndexEntry {
            resident: other,
            name: None,
        },
    ];
    assert!(s.regenerate_index(&entries));
    let index = std::fs::read_to_string(dir.0.join(INDEX_NAME)).unwrap();
    assert_eq!(
        index,
        format!("{}{},{RES},some.one,\"Some, One\"\n", csv::INDEX_HEADER, archive_name(RES))
    );
    assert_eq!(s.enumerate(), (vec![RES], true));

    let pending = [IndexEntry { resident: RES, name: None }];
    assert!(s.regenerate_index(&pending));
    let index = std::fs::read_to_string(dir.0.join(INDEX_NAME)).unwrap();
    assert!(index.ends_with(&format!(",{RES},{PENDING_NAME},{PENDING_NAME}\n")));

    std::fs::remove_file(dir.0.join(archive_name(RES))).unwrap();
    assert!(s.regenerate_index(&entries));
    assert!(!dir.0.join(INDEX_NAME).exists());
}

#[test]
fn a_delete_salvages_the_later_cutoff_and_keeps_only_the_state() {
    let dir = temp();
    let mut s = storage(&dir);
    s.publish_rows(RES, &[row(1)], false, ArchiveScan::default()).unwrap();
    std::fs::write(dir.0.join(format!("{}.corrupt", archive_name(RES))), b"x").unwrap();
    // a damaged state that still names a later cutoff
    std::fs::write(dir.0.join(STATE_NAME), format!("{}trailing", state_contents(900, false))).unwrap();
    assert_eq!(s.load_state(), StateLoad::Unsafe);

    assert_eq!(s.begin_delete_state(500), Some(900));
    assert_eq!(
        s.load_state(),
        StateLoad::Safe {
            deleted_before_ticks: 900,
            cleanup_pending: true
        }
    );
    assert!(!dir.0.join("chat_service_index.csv.tmp").exists());

    std::fs::write(dir.0.join("unrelated.txt"), b"keep").unwrap();
    assert!(s.sweep_service_artifacts());
    assert_eq!(listed(&dir.0), vec![STATE_NAME.to_string()]);
    assert!(dir.0.join("unrelated.txt").exists());

    assert!(s.write_state(900, false));
    assert_eq!(
        s.load_state(),
        StateLoad::Safe {
            deleted_before_ticks: 900,
            cleanup_pending: false
        }
    );
    // a later click wins over the stored cutoff
    assert_eq!(s.begin_delete_state(1000), Some(1000));
}
