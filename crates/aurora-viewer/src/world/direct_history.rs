//! The Chat Service's direct-IM history in the World: the manager's
//! inputs (login, region, gates, answers), its side effects (requests,
//! files, names, conversation log, deletion) and the history of every
//! direct conversation, stitched from the archive, the plaintext
//! transcript and the head page. Port of the glue of LLChatServiceHistory
//! and of its History view helper (indra/newview/llchatservicehistory.cpp),
//! of LLIMModel::LLIMSession's use of it (llimview.cpp) and of the live
//! capture in LLIMProcessing::processNewMessage (llimprocessing.cpp), LL
//! viewer 26.5, LGPL 2.1.
//!
//! - every file operation runs on the transcript thread (`aurora-chatlog`),
//!   after the transcript lines queued before it, and answers through a
//!   channel polled each frame;
//! - group chats and conferences keep the transcript's tail
//!   (`transcripts.rs`); so does every conversation while the manager is
//!   not running (no login yet, the offline tests);
//! - history lines are `Origin::Log`: never unread, never a toast or a
//!   sound, and the view draws no separator between them and the session.

use super::chat_log::Entry;
use super::chat_log::format;
use super::chat_log::service::csv::ArchiveScan;
use super::chat_log::service::merge::{History, Messages, replace_history};
use super::chat_log::service::message::{HistoryMessage, LiveContext, capture_live_message, incoming_context, record_live_message};
use super::chat_service::stitch::{self, PREVIEW_LIMIT, ReadResult, SESSION_LIMIT};
use super::chat_service::storage::{self, Storage};
use super::chat_service::{Action, Context, Env, Gates, Manager, PreparedArchive, StateLoad};
use super::names::{AvatarName, AvatarNames, NameScope};
use super::{ChatKind, ChatLine, Origin, World};
use aurora_net::NetCommand;
use crossbeam_channel::{Receiver, Sender};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// The region capabilities of the service (CapabilityContext).
pub const LIST_CAP: &str = "PersistentChatConversationsRequest";
pub const HISTORY_CAP: &str = "PersistentChatHistoryRequest";

/// The key a live row carries to find its session line again.
const LINE_KEY: &str = "aurora_line";

/// What the transcript thread answers.
enum Reply {
    State(u64, StateLoad),
    Archives(u64, Vec<Uuid>, bool),
    Prepared(u64, Option<PreparedArchive>),
    Published(u64, Option<ArchiveScan>),
    Index(u64, bool),
    DeleteState(u64, Option<u64>),
    Swept(u64, bool),
    StateWritten(u64, bool),
    /// A view's stitched read (`resident`, its token).
    Read(Uuid, u64, ReadResult),
}

/// How a live line was captured (the history context of its session row).
#[derive(Debug, Clone)]
struct Capture {
    time: SystemTime,
    text: String,
    context: LiveContext,
    timestamp: u32,
    logged_at: u32,
}

/// The history of one direct conversation, an IM session's or the History
/// window's (LLChatServiceHistory::History).
#[derive(Debug, Default)]
pub struct DirectView {
    history: History,
    token: u64,
    loading: bool,
    archive_serial: u32,
    service_allowed: bool,
    /// The transcript stem the plaintext is read from, once known.
    stem: Option<String>,
    /// The live lines' captures, oldest first.
    captures: Vec<Capture>,
    /// The last history placed, to place it once.
    placed: Option<Messages>,
    /// Snapshot::service_work_active at the last snapshot.
    work_active: bool,
    /// A stitched read was placed in the conversation.
    read_once: bool,
    /// The first history is shown, its fetch over: the later refreshes
    /// (an IM's quiet delay, the hourly list) run without the loading line.
    /// The LL viewer shows its loading panel on every pass; a small
    /// overlay there, a line over the box here (human choice, 2026-11-18).
    settled: bool,
}

impl DirectView {
    fn settle(&mut self) {
        if self.read_once && !self.loading && !self.work_active {
            self.settled = true;
        }
    }
}

/// The service's state in the World. Kept across logouts, like the
/// transcripts' writer: the manager's tickets and epochs never restart, so
/// an answer of a past login is never taken for one of this login.
#[derive(Default)]
pub struct ChatService {
    pub manager: Manager,
    /// ChatServiceEnabled (`chat_service_history`), read at login only.
    pub enabled: bool,
    /// Start from an empty folder (the demo's `service` scenario only).
    pub fresh_folder: bool,
    storage: Option<Arc<Mutex<Storage>>>,
    channel: Option<(Sender<Reply>, Receiver<Reply>)>,
    context: Option<Context>,
    gates: Gates,
    /// LogShowHistory at the last look: a change reloads the histories.
    show_history: Option<bool>,
    /// Names the manager waits for.
    names: HashSet<Uuid>,
    views: HashMap<Uuid, DirectView>,
    /// The History window's read, by resident.
    preview: Option<(Uuid, DirectView)>,
    preview_result: Option<(Uuid, Messages)>,
    /// The manager has inputs it has not run yet.
    dirty: bool,
    /// The last delete's outcome, for the Options page.
    delete_done: Option<bool>,
    /// Transcript files the last delete removed.
    swept: Arc<std::sync::atomic::AtomicUsize>,
    /// A delete closed the History window (invalidateHistory).
    pub close_preview: bool,
}

/// The World's half of the manager's queries (LLMuteList, LLAvatarNameCache).
struct WorldEnv<'a> {
    mutes: &'a super::mutes::MuteList,
    names: &'a AvatarNames,
}

impl Env for WorldEnv<'_> {
    fn is_muted(&self, id: Uuid) -> bool {
        self.mutes.is_muted_id(&id)
    }
    fn is_muted_name(&self, id: Uuid, user_name: &str) -> bool {
        self.mutes.is_muted(&id, user_name, 0)
    }
    fn cached_name(&self, id: Uuid) -> Option<AvatarName> {
        self.names.raw(&id).filter(|n| !n.temporary).cloned()
    }
}

fn unix_secs(t: SystemTime) -> u32 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0, |d| u32::try_from(d.as_secs()).unwrap_or(u32::MAX))
}

impl ChatService {
    fn sender(&mut self) -> Sender<Reply> {
        self.channel.get_or_insert_with(crossbeam_channel::unbounded).0.clone()
    }

    /// A direct conversation shows the stitched history.
    pub fn running(&self) -> bool {
        self.manager.is_running()
    }

    /// The IM window's loading panel: a read in flight, or remote, name or
    /// storage work for the resident (Snapshot::service_work_active), only
    /// while the history is shown (LogShowHistory, LLFloaterIMSession::draw).
    pub fn loading(&self, resident: Uuid) -> bool {
        self.running()
            && self.show_history != Some(false)
            && self
                .views
                .get(&resident)
                .is_some_and(|v| !v.settled && (v.loading || v.work_active))
    }

    /// The outcome of the last delete, once.
    pub fn take_deleted(&mut self) -> Option<bool> {
        self.delete_done.take()
    }

    /// The transcript files the last delete removed.
    pub fn swept_files(&self) -> usize {
        self.swept.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The History window's stitched transcript, once read.
    pub fn take_preview(&mut self, resident: Uuid) -> Option<Messages> {
        match self.preview_result.take() {
            Some((id, m)) if id == resident => Some(m),
            other => {
                self.preview_result = other;
                None
            }
        }
    }

    #[cfg(test)]
    pub fn view_loading(&self, resident: Uuid) -> bool {
        self.views.get(&resident).is_some_and(|v| v.loading)
    }
}

impl World {
    // ---- lifecycle ------------------------------------------------------

    /// Login complete (LLChatServiceHistory::start): the account's folder
    /// for the grid, and the manager runs.
    pub(super) fn chat_service_login(&mut self) {
        let agent = self.agent_id;
        if agent.is_nil() {
            return;
        }
        let grid = self.chat_log.identity().map(|i| i.grid.clone()).unwrap_or_default();
        let dir = storage::folder(&crate::settings::account_dir(&agent), &grid);
        self.chat_service_start_in(dir);
    }

    /// Start with the files in `dir` (the tests' folder).
    pub(super) fn chat_service_start_in(&mut self, dir: std::path::PathBuf) {
        let agent = self.agent_id;
        let cs = &mut self.chat_service;
        if cs.fresh_folder {
            // the demo account's own folder, before any job reads it
            let _ = std::fs::remove_dir_all(&dir);
        }
        cs.storage = Some(Arc::new(Mutex::new(Storage::new(dir, agent))));
        cs.views.clear();
        cs.preview = None;
        cs.names.clear();
        cs.context = None;
        cs.manager.start(agent, cs.enabled);
        cs.dirty = true;
    }

    /// Logout or disconnect (LLChatServiceHistory::stop).
    pub(super) fn chat_service_logout(&mut self) {
        let cs = &mut self.chat_service;
        cs.manager.stop();
        cs.storage = None;
        cs.views.clear();
        cs.preview = None;
        cs.preview_result = None;
        cs.names.clear();
        cs.context = None;
    }

    /// A direct conversation with a resident, under the service.
    pub(super) fn is_direct(&self, id: &Uuid) -> bool {
        self.chat_service.running() && !id.is_nil() && *id != self.agent_id && !self.is_group_session(id) && self.groups.group(id).is_none()
    }

    // ---- every frame ----------------------------------------------------

    /// Every frame: the region's context and the gates, the answers of the
    /// thread and of the name cache, then the manager when it has work.
    pub fn poll_chat_service(&mut self) {
        if !self.chat_service.running() {
            return;
        }
        let context = self.main().and_then(|r| {
            let region_id = r.info.as_ref()?.region_id;
            Some(Context {
                region_id,
                list_url: r.caps.get(LIST_CAP)?.clone(),
                history_url: r.caps.get(HISTORY_CAP)?.clone(),
            })
        });
        if context != self.chat_service.context {
            self.chat_service.manager.set_context(context.clone());
            self.chat_service.context = context;
            self.chat_service.dirty = true;
        }
        let gates = Gates {
            consent: self.chat_log.cfg.log_im,
            mute_list_server_loaded: self.mutes.is_loaded_from_server(),
        };
        if gates != self.chat_service.gates {
            self.chat_service.manager.set_gates(gates);
            self.chat_service.gates = gates;
            self.chat_service.dirty = true;
        }
        let show = self.chat_log.cfg.show_history;
        if self.chat_service.show_history.replace(show).is_some_and(|old| old != show) {
            self.chat_service.manager.show_history_changed();
            self.chat_service.dirty = true;
        }
        let resolved: Vec<(Uuid, AvatarName)> = self
            .chat_service
            .names
            .iter()
            .filter_map(|id| Some((*id, self.social.avatar_names.raw(id).filter(|n| !n.temporary)?.clone())))
            .collect();
        for (id, name) in resolved {
            self.chat_service.names.remove(&id);
            self.chat_service.manager.name_resolved(id, Some(name));
            self.chat_service.dirty = true;
        }
        let replies: Vec<Reply> = self
            .chat_service
            .channel
            .as_ref()
            .map(|(_, rx)| rx.try_iter().collect())
            .unwrap_or_default();
        for reply in replies {
            self.chat_service_reply(reply);
        }
        // sessions that closed take their view along
        let open: HashSet<Uuid> = self.social.ims.iter().map(|s| s.other).collect();
        self.chat_service.views.retain(|id, _| open.contains(id));
        self.run_chat_service();
    }

    /// Run the manager when an input or its clock asks, and its actions
    /// in order.
    fn run_chat_service(&mut self) {
        for _ in 0..8 {
            let now = Instant::now();
            let due = self.chat_service.manager.next_wake(now).is_some_and(|t| t <= now);
            if !self.chat_service.dirty && !due {
                return;
            }
            self.chat_service.dirty = false;
            let env = WorldEnv {
                mutes: &self.mutes,
                names: &self.social.avatar_names,
            };
            let actions = self.chat_service.manager.poll(now, &env);
            if actions.is_empty() {
                return;
            }
            for action in actions {
                self.chat_service_action(action);
            }
        }
    }

    /// The answer of an HTTP request (`NetEvent::ChatServiceResponse`).
    pub(super) fn chat_service_response(&mut self, id: u64, status: u16, body: Option<aurora_llsd::Llsd>) {
        self.chat_service.manager.on_response(id, status, body);
        self.chat_service.dirty = true;
        self.run_chat_service();
    }

    fn chat_service_reply(&mut self, reply: Reply) {
        let m = &mut self.chat_service.manager;
        match reply {
            Reply::State(t, s) => m.state_loaded(t, s),
            Reply::Archives(t, ids, any) => m.archives_listed(t, ids, any),
            Reply::Prepared(t, p) => m.archive_prepared(t, p),
            Reply::Published(t, s) => m.rows_published(t, s),
            Reply::Index(t, ok) => m.index_updated(t, ok),
            Reply::DeleteState(t, b) => m.delete_state_written(t, b),
            Reply::Swept(t, ok) => m.transcripts_swept(t, ok),
            Reply::StateWritten(t, ok) => m.state_written(t, ok),
            Reply::Read(id, token, result) => return self.direct_read_done(id, token, result),
        }
        self.chat_service.dirty = true;
    }

    /// One side effect of the manager.
    fn chat_service_action(&mut self, action: Action) {
        match action {
            Action::Http { id, url, body } => self.groups.out.push(NetCommand::ChatServiceRequest { id, url, body }),
            Action::LoadState { ticket } => self.storage_job(Reply::State(ticket, StateLoad::Unsafe), move |s| {
                Reply::State(ticket, s.load_state())
            }),
            Action::ListArchives { ticket } => self.storage_job(Reply::Archives(ticket, Vec::new(), false), move |s| {
                let (ids, any) = s.enumerate();
                Reply::Archives(ticket, ids, any)
            }),
            Action::PrepareArchive {
                ticket,
                resident,
                deleted_before_ticks,
            } => self.storage_job(Reply::Prepared(ticket, None), move |s| {
                Reply::Prepared(ticket, s.prepare_archive(resident, deleted_before_ticks))
            }),
            Action::PublishRows {
                ticket,
                resident,
                rows,
                append,
                expected,
            } => self.storage_job(Reply::Published(ticket, None), move |s| {
                Reply::Published(ticket, s.publish_rows(resident, &rows, append, expected))
            }),
            Action::UpdateIndex { ticket, entries } => self.storage_job(Reply::Index(ticket, false), move |s| {
                Reply::Index(ticket, s.regenerate_index(&entries))
            }),
            Action::BeginDeleteState { ticket, click_ticks } => self.storage_job(Reply::DeleteState(ticket, None), move |s| {
                Reply::DeleteState(ticket, s.begin_delete_state(click_ticks))
            }),
            Action::SweepTranscripts { ticket } => {
                // the plaintext first (deleteTranscriptContent), then the
                // service's files; no transcript folder is a failure
                let dir = self.chat_log.dir().map(std::path::Path::to_path_buf);
                let swept = self.chat_service.swept.clone();
                self.storage_job(Reply::Swept(ticket, false), move |s| {
                    let erased = dir.map(|d| super::chat_log::store::erase(&d));
                    if let Some(Ok(n)) = &erased {
                        swept.store(*n, std::sync::atomic::Ordering::Relaxed);
                    }
                    let plain = matches!(erased, Some(Ok(_)));
                    Reply::Swept(ticket, plain && s.sweep_service_artifacts())
                });
            }
            Action::WriteState {
                ticket,
                boundary,
                cleanup_pending,
            } => self.storage_job(Reply::StateWritten(ticket, false), move |s| {
                Reply::StateWritten(ticket, s.write_state(boundary, cleanup_pending))
            }),
            Action::ResolveName(id) => {
                self.social.avatar_names.want(&id);
                self.chat_service.names.insert(id);
            }
            Action::SnapshotChanged(id) => self.direct_on_snapshot(id),
            Action::ConversationLogEntry {
                resident,
                newest_unix_secs,
            } => self.log_service_conversation(resident, newest_unix_secs),
            // the menus read the "transcript exists" state when drawn
            Action::TranscriptCreated => {}
            Action::StyleUpdate => {
                let ids: Vec<Uuid> = self.chat_service.views.keys().copied().collect();
                for id in ids {
                    self.direct_load(id);
                }
            }
            Action::ClearSessionsForDeletion => self.clear_sessions_for_deletion(),
            Action::DeleteFinished { success } => {
                if !success {
                    self.notify_history_delete_failed();
                }
                self.chat_service.delete_done = Some(success);
            }
        }
    }

    /// Run `job` on the transcript thread with the account's storage; a
    /// job the thread refused (or no storage) is answered with `failed`
    /// at once, so the manager never waits for it.
    fn storage_job(&mut self, failed: Reply, job: impl FnOnce(&mut Storage) -> Reply + Send + 'static) {
        let tx = self.chat_service.sender();
        let Some(storage) = self.chat_service.storage.clone() else {
            self.chat_service_reply(failed);
            return;
        };
        let reply = tx.clone();
        let queued = self.chat_log.run_job(move || {
            let answer = match storage.lock() {
                Ok(mut s) => job(&mut s),
                Err(poisoned) => job(&mut poisoned.into_inner()),
            };
            let _ = reply.send(answer);
        });
        if !queued {
            self.chat_service_reply(failed);
        }
    }

    /// LLConversationLog::addServiceConversation.
    fn log_service_conversation(&mut self, resident: Uuid, newest_unix_secs: Option<f64>) {
        if !self.chat_log.conversations.logging {
            return;
        }
        let Some(mut c) = self.conversation_entry(resident) else {
            return;
        };
        if let Some(t) = newest_unix_secs {
            c.time = t as i64;
        }
        if self.social.avatar_names.complete_in(NameScope::Roster, &resident).is_none() {
            self.chat_log.conversations.unnamed.insert(resident);
        }
        self.chat_log.conversations.log_service(c);
    }

    /// Delete step 3: every IM session clearForHistoryDeletion (lines and
    /// unread counts), every view's history and the History window.
    fn clear_sessions_for_deletion(&mut self) {
        for s in &mut self.social.ims {
            s.lines.clear();
            s.unread = 0;
            s.clock.renumbered();
        }
        for v in self.chat_service.views.values_mut() {
            v.history.clear();
            v.captures.clear();
            v.placed = None;
            v.token += 1;
            v.loading = false;
        }
        self.chat_service.preview = None;
        self.chat_service.preview_result = None;
        self.chat_service.close_preview = true;
    }

    /// ChatServiceHistoryDeleteFailed, through the notices.
    fn notify_history_delete_failed(&mut self) {
        let text = aurora_i18n::tr!("chat-service-delete-failed").to_string();
        self.alerts.push_back(text);
    }

    // ---- deletion -------------------------------------------------------

    /// Delete Transcripts confirmed: through the manager while it runs
    /// (deleteTranscriptsAsync); false when it is not running or already
    /// deleting, and in the shared Legacy layout, whose files are the
    /// Legacy Viewer's too.
    pub fn request_history_delete(&mut self) -> bool {
        if !self.chat_service.running() || self.chat_log.shared() || self.chat_log.busy() {
            return false;
        }
        // time_corrected: Aurora reads the computer's UTC clock, as the
        // conversation log does
        let micros = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_micros()).unwrap_or(i64::MAX));
        let Some(ticks) = super::chat_service::click_ticks(micros) else {
            return false;
        };
        let started = self.chat_service.manager.request_delete(ticks);
        if started {
            self.chat_service.dirty = true;
            self.run_chat_service();
        }
        started
    }

    // ---- the views ------------------------------------------------------

    /// A direct session exists (it opened, or a line came): its view, the
    /// priority of its resident and its first read. Port of
    /// LLIMModel::newSession's prioritizeResident and History::load.
    pub(super) fn direct_open(&mut self, id: Uuid) {
        if !self.is_direct(&id) || self.chat_service.views.contains_key(&id) {
            return;
        }
        self.chat_service.views.insert(id, DirectView::default());
        let env = WorldEnv {
            mutes: &self.mutes,
            names: &self.social.avatar_names,
        };
        self.chat_service.manager.prioritize(id, false, &env);
        self.chat_service.dirty = true;
        self.direct_load(id);
    }

    /// History::load / reload: a stitched read of the resident's history.
    fn direct_load(&mut self, id: Uuid) {
        if !self.chat_log.cfg.show_history {
            // no history rows; the service still runs
            if let Some(v) = self.chat_service.views.get_mut(&id) {
                v.history.clear();
            }
            self.direct_place(id);
            return;
        }
        let snapshot = self.chat_service.manager.snapshot(id);
        let stem = self.direct_stem(id, snapshot.name.as_ref());
        let family = stem.as_deref().and_then(|s| self.chat_log.family_of(s));
        let env = WorldEnv {
            mutes: &self.mutes,
            names: &self.social.avatar_names,
        };
        let read = self.chat_service.manager.begin_view_read(id, Instant::now(), &env);
        let Some(v) = self.chat_service.views.get_mut(&id) else {
            return;
        };
        v.stem = stem;
        v.token += 1;
        v.archive_serial = snapshot.archive_serial;
        v.service_allowed = snapshot.service_presentation_allowed;
        let Some(read) = read else {
            // suppressed: nothing historical is shown
            v.history.clear();
            v.loading = false;
            self.direct_place(id);
            return;
        };
        v.loading = true;
        let token = v.token;
        self.chat_service.dirty = true;
        self.stitched_read(id, token, family, read, SESSION_LIMIT);
    }

    fn stitched_read(
        &mut self,
        id: Uuid,
        token: u64,
        family: Option<(std::path::PathBuf, String)>,
        read: super::chat_service::ViewRead,
        limit: usize,
    ) {
        let agent = self.agent_id;
        let tx = self.chat_service.sender();
        let Some(storage) = self.chat_service.storage.clone() else {
            return;
        };
        let queued = self.chat_log.run_job(move || {
            let result = match storage.lock() {
                Ok(s) => stitch::read_stitched(&s, agent, id, family, &read, limit),
                Err(p) => stitch::read_stitched(&p.into_inner(), agent, id, family, &read, limit),
            };
            let _ = tx.send(Reply::Read(id, token, result));
        });
        if !queued {
            // the thread is gone: plaintext only would need it too
            self.direct_read_done(id, token, ReadResult::default());
        }
    }

    /// A stitched read ended: dropped when another started since, read
    /// again when its inputs changed, else loaded and placed.
    fn direct_read_done(&mut self, id: Uuid, token: u64, result: ReadResult) {
        if result.maintenance_needed {
            self.chat_service.manager.view_read_damaged(result.epoch, id, result.archive_serial);
            self.chat_service.dirty = true;
        }
        let snapshot = self.chat_service.manager.snapshot(id);
        let suppressed = self.chat_service.manager.history_suppressed();
        let epoch = self.chat_service.manager.epoch();
        let stem = self.direct_stem(id, snapshot.name.as_ref());
        // the History window's read
        if let Some((pid, v)) = self.chat_service.preview.as_mut()
            && *pid == id
            && v.token == token
        {
            v.loading = false;
            if result.epoch != epoch || suppressed {
                return;
            }
            v.history.set_loaded(result.messages);
            let composed = v.history.compose(&[], &[], PREVIEW_LIMIT, false);
            self.chat_service.preview_result = Some((id, composed));
            return;
        }
        let Some(v) = self.chat_service.views.get_mut(&id) else {
            return;
        };
        if v.token != token {
            return;
        }
        v.loading = false;
        if result.epoch != epoch || suppressed {
            return;
        }
        if result.archive_serial != snapshot.archive_serial
            || result.included_service != snapshot.service_presentation_allowed
            || stem != v.stem
        {
            self.direct_load(id);
            return;
        }
        v.archive_serial = result.archive_serial;
        v.history.set_loaded(result.messages);
        v.read_once = true;
        v.settle();
        self.direct_place(id);
    }

    /// History::onSnapshot: a new preview is shown at once; a consent
    /// change, a name that names the transcript or a new archive reads
    /// again.
    fn direct_on_snapshot(&mut self, id: Uuid) {
        let snapshot = self.chat_service.manager.snapshot(id);
        if let Some(v) = self.chat_service.views.get_mut(&id) {
            v.work_active = snapshot.service_work_active;
            v.settle();
        }
        if self.chat_service.manager.history_suppressed() {
            return;
        }
        let stem = self.direct_stem(id, snapshot.name.as_ref());
        let Some(v) = self.chat_service.views.get_mut(&id) else {
            return;
        };
        let presentation_changed = v.service_allowed != snapshot.service_presentation_allowed;
        v.service_allowed = snapshot.service_presentation_allowed;
        let name_resolved = v.stem.is_none() && stem.is_some();
        let reload = !v.loading && (presentation_changed || name_resolved || snapshot.archive_serial != v.archive_serial);
        if presentation_changed || !snapshot.head_preview.is_empty() {
            self.direct_place(id);
        }
        if reload {
            self.direct_load(id);
        }
    }

    /// The stem of the resident's plaintext: the transcript's, else the
    /// legacy name the service resolved (LLCacheName::buildUsername).
    fn direct_stem(&self, id: Uuid, name: Option<&AvatarName>) -> Option<String> {
        self.transcript_stem(&id).or_else(|| {
            let n = name.filter(|n| !n.legacy_first.is_empty())?;
            Some(if n.legacy_last.is_empty() || n.legacy_last == "Resident" {
                n.legacy_first.clone()
            } else {
                format!("{} {}", n.legacy_first, n.legacy_last)
            })
        })
    }

    /// The session's live lines as session rows, newest first, each with
    /// its line's index and its capture.
    fn direct_live_rows(&self, id: Uuid) -> Messages {
        let Some(s) = self.social.ims.iter().find(|s| s.other == id) else {
            return Vec::new();
        };
        let captures = self.chat_service.views.get(&id).map_or(&[][..], |v| v.captures.as_slice());
        let mut rows: Messages = s
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.origin != Origin::Log)
            .map(|(i, l)| {
                let (from, text) = self.transcript_text(NameScope::Roster, l);
                let capture = captures.iter().rev().find(|c| c.time == l.time && c.text == l.text);
                let from_id = match l.kind {
                    ChatKind::Own(_) => Some(self.agent_id),
                    ChatKind::System => Some(Uuid::nil()),
                    _ => Some(l.source),
                };
                let mut row = HistoryMessage {
                    from,
                    from_id,
                    message: text,
                    time: format::stamp(l.time, true),
                    datetime: format::stamp(l.time, true),
                    timestamp: Some(capture.map_or(unix_secs(l.time), |c| c.timestamp) as i32),
                    ..HistoryMessage::default()
                };
                row.extra.insert(LINE_KEY.into(), aurora_llsd::Llsd::Integer(i as i32));
                if let Some(c) = capture {
                    record_live_message(&mut row, &c.context, c.logged_at);
                }
                row
            })
            .collect();
        rows.reverse();
        rows
    }

    /// Compose the view's history with the preview and the live rows, and
    /// put it into the session (replaceHistory): live lines keep their
    /// order, history lines are `Origin::Log`; the unread count is left
    /// alone.
    fn direct_place(&mut self, id: Uuid) {
        let live = self.direct_live_rows(id);
        let snapshot = self.chat_service.manager.snapshot(id);
        let show = self.chat_log.cfg.show_history;
        let Some(v) = self.chat_service.views.get_mut(&id) else {
            return;
        };
        let preview: Messages = if snapshot.service_presentation_allowed {
            snapshot.head_preview.iter().map(stitch::service_message).collect()
        } else {
            v.history.clear_service();
            Vec::new()
        };
        let composed = if show {
            v.history.compose(&preview, &live, SESSION_LIMIT, true)
        } else {
            Vec::new()
        };
        if v.placed.as_ref() == Some(&composed) {
            return;
        }
        v.placed = Some(composed.clone());
        let mut current = live;
        if !replace_history(&mut current, &composed, true) {
            return;
        }
        let peer = Some(id);
        let history_lines = self.service_lines(current.iter().rev().filter(|m| m.is_history), peer);
        let Some(s) = self.social.ims.iter_mut().find(|s| s.other == id) else {
            return;
        };
        let old: Vec<ChatLine> = s.lines.drain(..).collect();
        let mut history = history_lines.into_iter();
        for m in current.iter().rev() {
            if m.is_history {
                if let Some(l) = history.next() {
                    s.lines.push_back(l);
                }
            } else if let Some(aurora_llsd::Llsd::Integer(i)) = m.extra.get(LINE_KEY)
                && let Some(l) = old.get(*i as usize)
            {
                s.lines.push_back(l.clone());
            }
        }
        s.clock.renumbered();
    }

    /// History rows as conversation lines: ours, the system's or the
    /// sender's by id, by the cached legacy name, else the other resident
    /// (LLIMSession::addMessagesFromHistoryCache). A row without a time
    /// takes the one before.
    pub fn service_lines<'a>(&self, rows: impl Iterator<Item = &'a HistoryMessage>, peer: Option<Uuid>) -> Vec<ChatLine> {
        let names = &self.social.avatar_names;
        let trim = names.options.trim_resident;
        let own = self.own_log_names();
        let mut last = UNIX_EPOCH;
        rows.map(|m| {
            let time = match m.timestamp {
                Some(t) if t > 0 => Some(UNIX_EPOCH + Duration::from_secs(t as u64)),
                _ => format::parse_stamp(&m.datetime),
            };
            last = time.unwrap_or(last);
            let kind = if m.from_id == Some(Uuid::nil()) || m.from.is_empty() || m.from == format::SYSTEM_FROM {
                ChatKind::System
            } else if m.from_id == Some(self.agent_id) || own.contains(&m.from) {
                ChatKind::OWN
            } else {
                ChatKind::Im
            };
            let source = match kind {
                ChatKind::Im => m
                    .from_id
                    .or_else(|| names.find_id_by_name(&super::names::build_legacy_name(&m.from, trim)))
                    .or(peer)
                    .unwrap_or_default(),
                _ => Uuid::nil(),
            };
            ChatLine {
                time: last,
                from: if kind == ChatKind::System { String::new() } else { m.from.clone() },
                text: m.message.clone(),
                kind,
                source,
                origin: Origin::Log,
                blocked: false,
                beyond_range: false,
                speaker: Default::default(),
            }
        })
        .collect()
    }

    // ---- live messages --------------------------------------------------

    /// An incoming direct message entered its session (processNewMessage):
    /// its capture (the body before the "(Saved …)" notice, the send time
    /// offline, the receipt time online) and the resident's activity.
    pub(super) fn direct_incoming(&mut self, from: Uuid, line_time: SystemTime, line_text: &str, raw: &str, offline: bool, sent: u32) {
        if !self.is_direct(&from) {
            return;
        }
        let context = incoming_context(offline.then_some(raw), !offline);
        let mut timestamp = if offline { sent } else { 0 };
        let now = unix_secs(SystemTime::now());
        let context = capture_live_message(raw, &mut timestamp, &context, now);
        self.direct_capture(from, line_time, line_text, context, timestamp, now);
        let env = WorldEnv {
            mutes: &self.mutes,
            names: &self.social.avatar_names,
        };
        self.chat_service.manager.note_activity(from, Instant::now(), &env);
        self.chat_service.dirty = true;
    }

    /// The pre-send boundary of an outgoing message (History::outgoingContext),
    /// taken before it is sent.
    pub(super) fn direct_outgoing_context(&self, to: Uuid) -> Option<LiveContext> {
        if !self.is_direct(&to) {
            return None;
        }
        let latest = self
            .chat_service
            .views
            .get(&to)
            .map(|v| v.history.latest_service_key())
            .unwrap_or_default();
        Some(LiveContext {
            after_msg_id: Some(self.chat_service.manager.outgoing_after_msg_id(to, latest)),
            ..LiveContext::default()
        })
    }

    /// An outgoing IM_NOTHING_SPECIAL entered its session: its capture,
    /// then the resident's activity (sent).
    pub(super) fn direct_outgoing(&mut self, to: Uuid, context: LiveContext, line_time: SystemTime, line_text: &str) {
        let now = unix_secs(SystemTime::now());
        let mut timestamp = 0;
        let context = capture_live_message(line_text, &mut timestamp, &context, now);
        self.direct_capture(to, line_time, line_text, context, timestamp, now);
        let env = WorldEnv {
            mutes: &self.mutes,
            names: &self.social.avatar_names,
        };
        self.chat_service.manager.note_activity(to, Instant::now(), &env);
        self.chat_service.dirty = true;
    }

    fn direct_capture(&mut self, id: Uuid, time: SystemTime, text: &str, context: LiveContext, timestamp: u32, logged_at: u32) {
        self.direct_open(id);
        if let Some(v) = self.chat_service.views.get_mut(&id) {
            v.captures.push(Capture {
                time,
                text: text.to_owned(),
                context,
                timestamp,
                logged_at,
            });
            // the lines are bounded: so are their captures
            let excess = v.captures.len().saturating_sub(super::MAX_CHAT);
            v.captures.drain(..excess);
        }
    }

    // ---- the History window ---------------------------------------------

    /// The History window opened on a resident's transcript: its stitched
    /// history, 10000 rows (LLFloaterConversationPreview), and the
    /// resident first in the queue. False when the history is not the
    /// service's (not running, a group).
    pub fn open_direct_preview(&mut self, id: Uuid) -> bool {
        if !self.is_direct(&id) {
            return false;
        }
        self.chat_service.close_preview = false;
        let env = WorldEnv {
            mutes: &self.mutes,
            names: &self.social.avatar_names,
        };
        self.chat_service.manager.prioritize(id, false, &env);
        let read = self.chat_service.manager.begin_view_read(id, Instant::now(), &env);
        let stem = self.transcript_stem(&id);
        let family = stem.as_deref().and_then(|s| self.chat_log.family_of(s));
        self.chat_service.dirty = true;
        let token = self.chat_service.preview.as_ref().map_or(0, |(_, v)| v.token) + 1;
        let view = DirectView {
            token,
            loading: read.is_some(),
            stem,
            ..DirectView::default()
        };
        self.chat_service.preview = Some((id, view));
        self.chat_service.preview_result = None;
        match read {
            Some(read) => self.stitched_read(id, token, family, read, PREVIEW_LIMIT),
            None => self.chat_service.preview_result = Some((id, Vec::new())),
        }
        true
    }

    /// Transcript entries of rows, for the History window's pages.
    pub fn service_entries(rows: &[HistoryMessage]) -> Vec<Entry> {
        rows.iter()
            .map(|m| {
                let time = match m.timestamp {
                    Some(t) if t > 0 => Some(UNIX_EPOCH + Duration::from_secs(t as u64)),
                    _ => format::parse_stamp(&m.datetime),
                };
                Entry {
                    stamp: time.map_or_else(|| m.datetime.clone(), |t| format::stamp(t, true)),
                    time,
                    from: if m.from.is_empty() {
                        format::SYSTEM_FROM.to_owned()
                    } else {
                        m.from.clone()
                    },
                    text: m.message.clone(),
                }
            })
            .collect()
    }

    #[cfg(test)]
    pub(super) fn chat_service_settle(&mut self) {
        // each round answers what the last one queued
        for _ in 0..30 {
            assert!(self.chat_log.flush(Duration::from_secs(10)));
            self.poll_chat_service();
        }
    }
}

#[cfg(test)]
mod tests;
