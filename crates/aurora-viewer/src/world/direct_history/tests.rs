//! The direct-IM history in a World: the manager on a temporary folder,
//! the transcript thread for real, no network.

use super::*;
use crate::scene::avatar::AvatarLibrary;
use crate::world::chat_log;
use crate::world::chat_log::service::csv::ArchiveScan;
use crate::world::chat_log::service::message::TimeSource;
use crate::world::chat_service::storage::{STATE_NAME, archive_name};
use crate::world::groups::SessionInfo;
use aurora_net::InstantMessage;
use aurora_net::chat_service::{HistoryRow, TimeUuidKey, UUID_EPOCH, direct_conversation_id, format_time_uuid};
use std::path::{Path, PathBuf};

const ME: Uuid = Uuid::from_u128(1);
const JANE: Uuid = Uuid::from_u128(2);

fn temp() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aurora-direct-history-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn service_dir(root: &Path) -> PathBuf {
    root.join("chat_service")
}

fn world(root: &Path) -> World {
    let mut w = World::new(Arc::new(AvatarLibrary::load()));
    w.chat_log = chat_log::ChatLog::in_folder(root.join("accounts"));
    w.chat_log.set_grid("agni".into());
    w.agent_id = ME;
    w.chat_log.login(ME);
    w.chat_service.enabled = true;
    w.chat_service_start_in(service_dir(root));
    w
}

fn im(text: &str, offline: bool, timestamp: u32) -> InstantMessage {
    InstantMessage {
        from_agent_id: JANE,
        from_name: "Jane Doe".into(),
        to_agent_id: ME,
        dialog: 0,
        session_id: JANE,
        message: text.into(),
        offline,
        from_group: false,
        binary_bucket: Vec::new(),
        timestamp,
    }
}

/// 2026-08-12 12:00 UTC.
const BASE: u64 = 1_786_536_000;

/// Row `n`, `n` seconds after `BASE`.
fn row(n: u64, text: &str) -> HistoryRow {
    let key = TimeUuidKey {
        ticks: UUID_EPOCH + (BASE + n) * 10_000_000,
        tail: [i8::MIN, 0, 0, 0, 0, 0, 0, 0],
    };
    let created = chrono::DateTime::from_timestamp((BASE + n) as i64, 0).unwrap();
    HistoryRow {
        conversation_id: direct_conversation_id(ME, JANE),
        msg_id: format_time_uuid(&key),
        key,
        from_id: JANE,
        from_name: "Jane Doe".into(),
        message: text.into(),
        dialog: 0,
        created_at: created.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    }
}

fn archive(root: &Path, rows: &[HistoryRow]) {
    let mut s = Storage::new(service_dir(root), ME);
    s.publish_rows(JANE, rows, false, ArchiveScan::default()).expect("archive written");
}

fn texts(w: &World) -> Vec<(String, Origin)> {
    w.social.ims[0].lines.iter().map(|l| (l.text.clone(), l.origin)).collect()
}

fn clean(w: &World, root: &Path) {
    assert!(w.chat_log.flush(Duration::from_secs(10)));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_archive_comes_back_as_history_without_touching_unread() {
    let root = temp();
    archive(&root, &[row(1, "from the service"), row(2, "and again")]);
    let mut w = world(&root);
    w.on_im(im("hello", false, 0));
    w.chat_service_settle();
    w.poll_transcripts();
    w.chat_service_settle();
    assert_eq!(
        texts(&w),
        [
            ("from the service".to_owned(), Origin::Log),
            ("and again".to_owned(), Origin::Log),
            ("hello".to_owned(), Origin::Live)
        ]
    );
    let s = &w.social.ims[0];
    assert_eq!(s.unread, 1, "history is never unread");
    assert_eq!(s.lines[0].source, JANE);
    assert!(!w.chat_service.view_loading(JANE));
    clean(&w, &root);
}

#[test]
fn history_off_shows_no_rows_and_the_manager_still_runs() {
    let root = temp();
    archive(&root, &[row(1, "from the service")]);
    let mut w = world(&root);
    w.chat_log.cfg.show_history = false;
    w.on_im(im("hello", false, 0));
    w.chat_service_settle();
    assert_eq!(texts(&w), [("hello".to_owned(), Origin::Live)]);
    assert!(w.chat_service.running());
    clean(&w, &root);
}

#[test]
fn an_offline_message_is_captured_at_its_send_time_with_its_own_text() {
    let root = temp();
    let mut w = world(&root);
    w.on_im(im("while you were away", true, BASE as u32));
    w.chat_service_settle();
    let line = &w.social.ims[0].lines[0];
    assert!(line.text.ends_with("while you were away") && line.text != "while you were away");
    let c = &w.chat_service.views[&JANE].captures[0];
    assert_eq!(c.context.original_text.as_deref(), Some("while you were away"));
    assert_eq!(c.context.time_source, Some(TimeSource::Sent));
    assert_eq!(c.timestamp, BASE as u32);
    // online: the receipt time, the text as shown
    w.on_im(im("now", false, 0));
    let c = w.chat_service.views[&JANE].captures.last().unwrap();
    assert_eq!(c.context.time_source, Some(TimeSource::Receipt));
    assert_eq!(c.context.original_text.as_deref(), Some("now"));
    assert!(c.timestamp > 0);
    clean(&w, &root);
}

#[test]
fn an_outgoing_message_names_the_newest_service_id_before_it() {
    let root = temp();
    archive(&root, &[row(1, "from the service")]);
    let mut w = world(&root);
    w.on_im(im("hello", false, 0));
    w.chat_service_settle();
    w.own_im(JANE, "hi");
    let c = w.chat_service.views[&JANE].captures.last().unwrap().clone();
    assert_eq!(c.text, "hi");
    assert_eq!(c.context.after_msg_id, Some(row(1, "").msg_id));
    clean(&w, &root);
}

#[test]
fn only_direct_conversations_are_the_services() {
    let root = temp();
    let mut w = world(&root);
    let group = Uuid::from_u128(0x6000);
    w.groups.sessions.insert(group, SessionInfo::default());
    assert!(!w.is_direct(&group));
    assert!(w.is_direct(&JANE));
    assert!(!w.is_direct(&ME));
    w.chat_service_logout();
    assert!(!w.is_direct(&JANE), "not running: the transcript's tail");
    clean(&w, &root);
}

#[test]
fn a_delete_runs_every_step_through_the_transcript_thread() {
    let root = temp();
    archive(&root, &[row(1, "from the service")]);
    let mut w = world(&root);
    w.on_im(im("hello", false, 0));
    w.chat_service_settle();
    let transcript = w.chat_log.file_of("Jane Doe").unwrap();
    assert!(transcript.exists());
    assert!(w.request_history_delete());
    w.chat_service_settle();
    assert_eq!(w.chat_service.take_deleted(), Some(true));
    assert!(!transcript.exists(), "the plaintext is swept");
    let dir = service_dir(&root);
    assert!(!dir.join(archive_name(JANE)).exists(), "the archive is swept");
    let state = std::fs::read_to_string(dir.join(STATE_NAME)).unwrap();
    assert!(state.contains("<boolean>0</boolean>"), "cleanup is no longer pending: {state}");
    let s = &w.social.ims[0];
    assert!(s.lines.is_empty());
    assert_eq!(s.unread, 0);
    assert!(w.chat_service.close_preview);
    clean(&w, &root);
}
