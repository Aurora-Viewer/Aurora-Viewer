//! The direct-IM history in a World: the manager on a temporary folder,
//! the transcript thread for real, no network.

use super::*;
use crate::scene::avatar::AvatarLibrary;
use crate::world::chat_log;
use crate::world::chat_log::service::csv::ArchiveScan;
use crate::world::chat_log::service::message::TimeSource;
use crate::world::chat_service::storage::{STATE_NAME, archive_name};
use crate::world::groups::SessionInfo;
use crate::world::mutes::{Mute, MuteType};
use crate::world::status::AUTO_RESPONSE;
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

/// Jane's automatic response (IM_DO_NOT_DISTURB_AUTO_RESPONSE).
fn auto_response(text: &str) -> InstantMessage {
    InstantMessage {
        dialog: AUTO_RESPONSE,
        ..im(text, false, 0)
    }
}

/// Row `n`, `n` seconds after `BASE`.
fn row(n: u64, text: &str) -> HistoryRow {
    row_at(BASE + n, text, 0)
}

/// A row of Jane's recorded at `secs` (UTC seconds), of IM dialog `dialog`.
fn row_at(secs: u64, text: &str, dialog: u8) -> HistoryRow {
    let key = TimeUuidKey {
        ticks: UUID_EPOCH + secs * 10_000_000,
        tail: [i8::MIN, 0, 0, 0, 0, 0, 0, 0],
    };
    let created = chrono::DateTime::from_timestamp(secs as i64, 0).unwrap();
    HistoryRow {
        conversation_id: direct_conversation_id(ME, JANE),
        msg_id: format_time_uuid(&key),
        key,
        from_id: JANE,
        from_name: "Jane Doe".into(),
        message: text.into(),
        dialog: dialog.into(),
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

/// An automatic response's live line: `text` after its translated
/// "(auto-response)" prefix.
fn assert_auto_response_line(line: &(String, Origin), text: &str) {
    assert_eq!(line.1, Origin::Live, "{line:?}");
    assert!(line.0.ends_with(text) && line.0 != text, "the prefix is kept: {line:?}");
}

#[test]
fn an_automatic_response_and_its_service_row_show_once() {
    let root = temp();
    // the service records the response a few seconds after it arrived
    let now = u64::from(grid_now());
    archive(&root, &[row(1, "from the service"), row_at(now + 3, "Busy", AUTO_RESPONSE)]);
    let mut w = world(&root);
    w.on_im(auto_response("Busy"));
    w.chat_service_settle();
    w.poll_transcripts();
    w.chat_service_settle();
    let lines = texts(&w);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[0], ("from the service".to_owned(), Origin::Log));
    assert_auto_response_line(&lines[1], "Busy");
    // captured like an ordinary IM: the raw body, the receipt clock
    let c = &w.chat_service.views[&JANE].captures[0];
    assert_eq!(c.context.original_text.as_deref(), Some("Busy"));
    assert_eq!(c.context.time_source, Some(TimeSource::Receipt));
    clean(&w, &root);
}

#[test]
fn an_ordinary_im_and_an_automatic_response_with_the_same_text_take_one_row_each() {
    let root = temp();
    let now = u64::from(grid_now());
    archive(
        &root,
        &[
            // the same words months before: neither takes it
            row(1, "Busy"),
            row_at(now + 2, "Busy", 0),
            row_at(now + 3, "Busy", AUTO_RESPONSE),
        ],
    );
    let mut w = world(&root);
    w.on_im(im("Busy", false, 0));
    w.on_im(auto_response("Busy"));
    w.chat_service_settle();
    w.poll_transcripts();
    w.chat_service_settle();
    let lines = texts(&w);
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(lines[0], ("Busy".to_owned(), Origin::Log));
    assert_eq!(lines[1], ("Busy".to_owned(), Origin::Live));
    assert_auto_response_line(&lines[2], "Busy");
    clean(&w, &root);
}

#[test]
fn a_blocked_resident_s_automatic_response_is_neither_shown_nor_captured() {
    let root = temp();
    let mut w = world(&root);
    let jane = Mute {
        id: JANE,
        name: "Jane Doe".into(),
        kind: MuteType::Agent,
        flags: 0,
    };
    w.mutes.add(jane, 0).expect("blocked");
    // dropped before its own handling, as LLIMProcessing::processNewMessage
    // returns on is_muted
    w.on_im(auto_response("Busy"));
    w.chat_service_settle();
    assert!(!w.social.has_session(&JANE));
    assert!(!w.chat_service.views.contains_key(&JANE));
    assert!(!w.social.auto_responded.contains(&JANE));
    clean(&w, &root);
}

/// A row of ours recorded at `secs` (UTC seconds), `tail` telling apart
/// rows of the same second.
fn own_row_at(secs: u64, text: &str, tail: i8) -> HistoryRow {
    let mut row = row_at(secs, text, 0);
    row.key.tail[1] = tail;
    row.msg_id = format_time_uuid(&row.key);
    row.from_id = ME;
    row.from_name = "Me Resident".into();
    row
}

/// About 1,428 bytes of words, as in the report of #4875: two IMs.
fn long_text() -> String {
    (0..238).map(|i| format!("w{i:04}")).collect::<Vec<_>>().join(" ")
}

#[test]
fn a_long_sent_im_and_the_service_rows_of_its_parts_show_once() {
    let root = temp();
    let text = long_text();
    let parts = aurora_net::message_split::message_parts(&text);
    assert_eq!(parts.len(), 2, "{} bytes", text.len());
    // the service records each part a moment after it left
    let now = u64::from(grid_now());
    archive(
        &root,
        &[
            row(1, "from the service"),
            own_row_at(now + 2, parts[0], 1),
            own_row_at(now + 2, parts[1], 2),
        ],
    );
    let mut w = world(&root);
    w.own_im(JANE, &text);
    let c = w.chat_service.views[&JANE].captures.last().unwrap();
    assert_eq!(c.parts, parts);
    w.chat_service_settle();
    w.poll_transcripts();
    w.chat_service_settle();
    assert_eq!(
        texts(&w),
        [("from the service".to_owned(), Origin::Log), (text, Origin::Live)],
        "the line once, live, and neither part as history"
    );
    clean(&w, &root);
}

#[test]
fn a_sent_im_of_one_full_message_keeps_one_capture_body() {
    let root = temp();
    let text = "x".repeat(aurora_net::message_split::MAX_MESSAGE_BYTES);
    let now = u64::from(grid_now());
    archive(&root, &[row(1, "from the service"), own_row_at(now + 2, &text, 1)]);
    let mut w = world(&root);
    w.own_im(JANE, &text);
    assert!(w.chat_service.views[&JANE].captures.last().unwrap().parts.is_empty());
    w.chat_service_settle();
    w.poll_transcripts();
    w.chat_service_settle();
    assert_eq!(texts(&w), [("from the service".to_owned(), Origin::Log), (text, Origin::Live)]);
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

/// The account's own name, never the resident's: its Legacy Viewer folder
/// is `ada_example`.
const ACCOUNT: Option<(&str, &str)> = Some(("Ada", "Example"));

/// A World logged in with the account's name, the Legacy layout on or off,
/// in the app's order: the transcripts' folder, then the service.
fn named_world(root: &Path, shared: bool) -> World {
    let mut w = World::new(Arc::new(AvatarLibrary::load()));
    w.chat_log = chat_log::ChatLog::in_folder(root.join("accounts"));
    w.chat_log.set_grid("agni".into());
    w.chat_log.set_legacy_layout(shared);
    w.agent_id = ME;
    w.chat_log.login_named(ME, ACCOUNT);
    w.chat_service.enabled = true;
    w.chat_service_start_in(service_dir(root));
    w
}

/// A delete that did not finish (a crash, or a sweep that failed): the
/// archive and the pending cutoff, written by runDelete's first step.
fn pending_delete(root: &Path) {
    archive(root, &[row(1, "from the service")]);
    let mut s = Storage::new(service_dir(root), ME);
    assert!(s.begin_delete_state(UUID_EPOCH + (BASE + 10) * 10_000_000).is_some());
}

/// What the Legacy Viewer wrote in the account's folder: both spellings of
/// the resident's transcript.
fn legacy_folder(root: &Path) -> PathBuf {
    let dir = root.join("accounts").join("ada_example");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Jane Doe.txt"), "[2026/10/11 09:00]  Jane Doe: the legacy spelling\r\n").unwrap();
    std::fs::write(dir.join("jane_doe.txt"), "[2026/10/12 10:00]  Jane Doe: the username spelling\r\n").unwrap();
    dir
}

/// Every file of a folder with its bytes, by name.
fn listing(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap()))
        .collect();
    files.sort();
    files
}

/// The delete finished on the service's own files only, without a notice.
fn assert_finished_on_service_files(w: &mut World, root: &Path) {
    assert_eq!(w.chat_service.take_deleted(), Some(true));
    assert_eq!(w.chat_service.swept_files(), 0, "no transcript removed");
    let dir = service_dir(root);
    assert!(!dir.join(archive_name(JANE)).exists(), "the archive is swept");
    let state = std::fs::read_to_string(dir.join(STATE_NAME)).unwrap();
    assert!(state.contains("<boolean>0</boolean>"), "cleanup is no longer pending: {state}");
    let failed = aurora_i18n::tr!("chat-service-delete-failed").to_string();
    assert!(!w.alerts.contains(&failed));
}

#[test]
fn a_delete_resumed_in_a_shared_folder_keeps_every_legacy_transcript() {
    let root = temp();
    pending_delete(&root);
    let legacy = legacy_folder(&root);
    let before = listing(&legacy);
    let mut w = named_world(&root, true);
    assert!(w.chat_log.shared());
    assert_eq!(w.chat_log.dir(), Some(legacy.as_path()));
    w.chat_service_settle();
    assert_finished_on_service_files(&mut w, &root);
    assert_eq!(listing(&legacy), before, "the Legacy Viewer's folder is left as it was");
    clean(&w, &root);
}

#[test]
fn a_delete_that_reaches_a_shared_folder_keeps_every_transcript() {
    let root = temp();
    archive(&root, &[row(1, "from the service")]);
    let legacy = legacy_folder(&root);
    let before = listing(&legacy);
    let mut w = named_world(&root, false);
    let own = w.chat_log.file_of("Jane Doe").unwrap();
    std::fs::create_dir_all(own.parent().unwrap()).unwrap();
    std::fs::write(&own, "[2026/10/13 11:00]  Jane Doe: in Aurora's folder\r\n").unwrap();
    assert!(w.request_history_delete());
    // the folder is shared before the sweep (Preferences › Chat): the sweep
    // takes the folder placed then, and keeps its transcripts
    w.chat_log.set_legacy_layout(true);
    assert!(w.chat_log.shared());
    w.chat_service_settle();
    assert_finished_on_service_files(&mut w, &root);
    assert_eq!(listing(&legacy), before, "the Legacy Viewer's folder is left as it was");
    assert!(own.exists(), "the sweep never goes back to the folder of the click");
    assert!(!w.request_history_delete(), "no new delete while sharing");
    clean(&w, &root);
}

#[test]
fn a_delete_resumed_in_aurora_s_own_folder_still_erases_its_transcripts() {
    let root = temp();
    pending_delete(&root);
    let legacy = legacy_folder(&root);
    let before = listing(&legacy);
    let mut w = named_world(&root, false);
    assert!(!w.chat_log.shared());
    let own = w.chat_log.file_of("Jane Doe").unwrap();
    std::fs::create_dir_all(own.parent().unwrap()).unwrap();
    std::fs::write(&own, "[2026/10/13 11:00]  Jane Doe: in Aurora's folder\r\n").unwrap();
    w.chat_service_settle();
    assert_eq!(w.chat_service.take_deleted(), Some(true));
    assert!(!own.exists(), "the plaintext is swept");
    assert_eq!(w.chat_service.swept_files(), 1);
    assert!(!service_dir(&root).join(archive_name(JANE)).exists(), "the archive is swept");
    assert_eq!(listing(&legacy), before, "a folder not placed is never swept");
    clean(&w, &root);
}

#[test]
fn the_loading_line_waits_for_the_first_read_and_the_first_fetch() {
    let mut v = DirectView {
        loading: true,
        ..Default::default()
    };
    v.settle();
    assert!(!v.settled, "the read is in flight");
    v.loading = false;
    v.read_once = true;
    v.work_active = true;
    v.settle();
    assert!(!v.settled, "the first fetch runs");
    v.work_active = false;
    v.settle();
    assert!(v.settled);
    v.work_active = true;
    v.settle();
    assert!(v.settled, "a later refresh keeps it settled");
}

#[test]
fn a_refresh_after_the_first_history_shows_no_loading_line() {
    let root = temp();
    archive(&root, &[row(1, "from the service")]);
    let mut w = world(&root);
    w.on_im(im("hello", false, 0));
    w.chat_service_settle();
    w.poll_transcripts();
    w.chat_service_settle();
    assert!(!w.chat_service.loading(JANE));
    // an IM's quiet-delay refresh starts a pass
    w.chat_service.views.get_mut(&JANE).expect("the session's view").work_active = true;
    assert!(!w.chat_service.loading(JANE));
    clean(&w, &root);
}
