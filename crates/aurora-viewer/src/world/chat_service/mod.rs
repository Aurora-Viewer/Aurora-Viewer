//! The Chat Service's direct-IM history manager: the account-wide
//! scheduler that lists the direct conversations from the region's
//! capability, pages each one's history newest to oldest and has it
//! appended to the resident's CSV archive. Port of the scheduling half of
//! LLChatServiceHistory (indra/newview/llchatservicehistory.cpp, LL viewer
//! 26.5, LGPL 2.1), as a sans-I/O state machine.
//!
//! The LL viewer runs one coroutine per login (`manager`) that suspends on
//! HTTP responses, on file jobs of the General queue and on main-loop
//! work. Here every such suspension is a stop of the machine: it emits an
//! [`Action`] carrying a ticket and waits for the reply input with that
//! ticket; the checks the coroutine makes after resuming are made when the
//! reply arrives, in the same order. A reply whose ticket is not awaited
//! any more (logout, or a later login) is dropped, which is what the
//! coroutine's epoch check did. Time is passed in as `now`: nothing here
//! reads a clock, spawns a thread or touches the disk.
//!
//! - `manager`: the runtime state, the inputs, the queries, the manager
//!   loop and the storage maintenance turns;
//! - `sync`: the conversation list request and one resident's pass;
//! - `delete`: the transcript deletion and its recovery after a crash.

mod delete;
mod manager;
pub mod stitch;
pub mod storage;
mod sync;

#[cfg(test)]
mod tests;

pub use manager::Manager;

use crate::world::chat_log::service::csv::ArchiveScan;
use crate::world::names::AvatarName;
use aurora_llsd::Llsd;
use aurora_net::chat_service::{HistoryRow, UUID_EPOCH, UUID_TICK_LIMIT};
use std::time::Duration;
use uuid::Uuid;

/// Minimum gap between the starts of any two requests, list or history
/// (REQUEST_SPACING).
pub(super) const REQUEST_SPACING: Duration = Duration::from_millis(2100);
/// Delay before the one retry of a failed request, and before the list
/// that confirms an unknown conversation (RETRY_DELAY).
pub(super) const RETRY_DELAY: Duration = Duration::from_secs(5);
/// Quiet time after a resident's last IM before its history is refreshed
/// (ACTIVITY_REFRESH_DELAY).
pub(super) const ACTIVITY_REFRESH_DELAY: Duration = Duration::from_secs(15);
/// Account-wide cooldown after an HTTP 429 (RATE_LIMIT_DELAY).
pub(super) const RATE_LIMIT_DELAY: Duration = Duration::from_secs(120);
/// Period of the conversation list (LIST_INTERVAL).
pub(super) const LIST_INTERVAL: Duration = Duration::from_secs(3600);
/// How long a resident's name lookup may take (NAME_TIMEOUT).
pub(super) const NAME_TIMEOUT: Duration = Duration::from_secs(30);
/// The shortest wait of the coroutine (`llmax(0.01, …)`).
pub(super) const MIN_WAIT: Duration = Duration::from_millis(10);

/// The two capabilities of the agent's region (CapabilityContext). The
/// region id is part of it, so every region change restarts the discovery.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Context {
    pub region_id: Uuid,
    /// PersistentChatConversationsRequest.
    pub list_url: String,
    /// PersistentChatHistoryRequest.
    pub history_url: String,
}

impl Context {
    pub fn complete(&self) -> bool {
        !self.region_id.is_nil() && !self.list_url.is_empty() && !self.history_url.is_empty()
    }
}

/// The external gates the manager reads (transcriptConsent and
/// LLMuteList::isLoadedFromServer).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Gates {
    /// KeepConversationLogTranscripts > 1: transcripts are kept.
    pub consent: bool,
    /// The mute list came from the server (or the server said to use the
    /// cached one); a fallback cache after a timeout is not enough.
    pub mute_list_server_loaded: bool,
}

/// What the manager asks the World synchronously, where the LL viewer
/// queried a singleton (LLMuteList, LLAvatarNameCache).
pub trait Env {
    /// LLMuteList::isMuted(id).
    fn is_muted(&self, id: Uuid) -> bool;
    /// LLMuteList::isMuted(id, name): by id or by the legacy user name
    /// (LLAvatarName::getUserName).
    fn is_muted_name(&self, id: Uuid, user_name: &str) -> bool;
    /// LLAvatarNameCache::get(id, &name): a name already in the cache.
    fn cached_name(&self, id: Uuid) -> Option<AvatarName>;
}

/// The answer to [`Action::LoadState`] (loadState).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateLoad {
    /// A valid `chat_service_state.xml`, or neither it nor its `.tmp`.
    Safe { deleted_before_ticks: u64, cleanup_pending: bool },
    /// Anything ambiguous: all history stays hidden until a delete.
    Unsafe,
}

/// A successful [`Action::PrepareArchive`] (ArchivePreparation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedArchive {
    /// The scan, with no display rows.
    pub summary: ArchiveScan,
    /// A torn tail was cut or a corrupt file set aside.
    pub bytes_changed: bool,
}

/// One row of the index file: a resident with a valid archive and its
/// name when resolved ("(name pending)" otherwise).
#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub resident: Uuid,
    pub name: Option<AvatarName>,
}

/// What a view of one resident needs (LLChatServiceHistory::Snapshot).
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// Remote, name or storage work is still going on for the resident:
    /// the IM window shows its loading panel.
    pub service_work_active: bool,
    /// Service rows may be shown under the account-wide gates.
    pub service_presentation_allowed: bool,
    /// Bumped by every archive change: a read made before is stale.
    pub archive_serial: u32,
    pub name: Option<AvatarName>,
    /// The validated head page of the running pass, newest first, shown
    /// before it reaches the archive.
    pub head_preview: Vec<HistoryRow>,
}

/// The inputs of a stitched read, captured when it starts
/// (loadStitchedHistory).
#[derive(Debug, Clone)]
pub struct ViewRead {
    pub epoch: u32,
    /// Read the archive and add the preview rows; otherwise plaintext only.
    pub include_service: bool,
    pub archive_serial: u32,
    pub deleted_before_ticks: u64,
    /// The head preview, newest first (empty without `include_service`).
    pub preview: Vec<HistoryRow>,
}

/// A side effect for the World. The ones with a `ticket` are awaited:
/// the World answers with the matching input and the same ticket.
#[derive(Debug, Clone)]
pub enum Action {
    /// `NetCommand::ChatServiceRequest`: a GET of the list without a body,
    /// a POST of a history page with one; answered by
    /// [`Manager::on_response`].
    Http { id: u64, url: String, body: Option<Llsd> },
    /// Read the state file; answered by [`Manager::state_loaded`].
    LoadState { ticket: u64 },
    /// List the account's `chat_service_*` files (enumerateArchives);
    /// answered by [`Manager::archives_listed`].
    ListArchives { ticket: u64 },
    /// Scan, repair or set aside one archive (prepareArchive); answered by
    /// [`Manager::archive_prepared`].
    PrepareArchive {
        ticket: u64,
        resident: Uuid,
        deleted_before_ticks: u64,
    },
    /// Write the rows of a complete pass, oldest first: appended to a
    /// valid archive, or a new file replacing it (publishRows). The file
    /// must still be the one `expected` describes; answered by
    /// [`Manager::rows_published`].
    PublishRows {
        ticket: u64,
        resident: Uuid,
        rows: Vec<HistoryRow>,
        append: bool,
        expected: ArchiveScan,
    },
    /// Rewrite the index file (regenerateIndex); answered by
    /// [`Manager::index_updated`].
    UpdateIndex { ticket: u64, entries: Vec<IndexEntry> },
    /// Look the name up (LLAvatarNameCache::get with a callback); the
    /// answer is [`Manager::name_resolved`].
    ResolveName(Uuid),
    /// The resident's [`Snapshot`] changed: its views compose again.
    SnapshotChanged(Uuid),
    /// LLConversationLog::addServiceConversation for the resident, at the
    /// archive's newest time (`None`: now). Only when conversation logging
    /// is on.
    ConversationLogEntry { resident: Uuid, newest_unix_secs: Option<f64> },
    /// LLLogChat::notifyTranscriptCreated: the "transcript exists" state of
    /// the menus and buttons may have changed.
    TranscriptCreated,
    /// LLFloaterIMSessionTab::processChatHistoryStyleUpdate(true): reload
    /// the history of every direct session, then the open IM windows.
    StyleUpdate,
    /// Delete step 1: write `{max(click, salvaged cutoffs), pending}`
    /// durably, recovering an unreadable state file
    /// (recoverStateForDelete); answered by [`Manager::delete_state_written`]
    /// with the boundary written.
    BeginDeleteState { ticket: u64, click_ticks: u64 },
    /// Delete step 3, on the main thread before the sweep: every IM
    /// session clearForHistoryDeletion (history and live rows, unread
    /// counts), every conversation preview invalidated and closed. Run in
    /// order with the actions that follow it.
    ClearSessionsForDeletion,
    /// Delete step 4: the plaintext transcripts and their backups, then
    /// every `chat_service_*` artifact but the state file (and its
    /// `.tmp`); answered by [`Manager::transcripts_swept`].
    SweepTranscripts { ticket: u64 },
    /// Delete step 5: the final durable state; answered by
    /// [`Manager::state_written`].
    WriteState { ticket: u64, boundary: u64, cleanup_pending: bool },
    /// The end of a delete (finishDelete): on failure the World shows
    /// ChatServiceHistoryDeleteFailed; either way the Delete button comes
    /// back.
    DeleteFinished { success: bool },
}

/// The inclusive deletion cutoff of a click, the last UUIDv1 tick of its
/// microsecond, from the corrected UTC time in microseconds since 1970;
/// `None` out of range. Port of clickTicks (CS:2092-2116).
pub fn click_ticks(corrected_unix_micros: i64) -> Option<u64> {
    let micros = u64::try_from(corrected_unix_micros).ok()?;
    if micros > (UUID_TICK_LIMIT - 1 - UUID_EPOCH - 9) / 10 {
        return None;
    }
    let ticks = UUID_EPOCH + micros * 10 + 9;
    (ticks < UUID_TICK_LIMIT).then_some(ticks)
}
