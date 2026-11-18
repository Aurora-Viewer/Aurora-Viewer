//! The manager's state, its inputs and queries, and the manager loop.
//! Port of LLChatServiceHistory's runtime and of its `manager` coroutine
//! (indra/newview/llchatservicehistory.cpp, LL viewer 26.5).

use super::sync::Pass;
use super::{
    ACTIVITY_REFRESH_DELAY, Action, Context, Env, Gates, IndexEntry, LIST_INTERVAL, MIN_WAIT, NAME_TIMEOUT, PreparedArchive, Snapshot,
    StateLoad, ViewRead,
};
use crate::world::chat_log::service::csv::{ArchiveScan, ArchiveState};
use crate::world::names::AvatarName;
use aurora_llsd::Llsd;
use aurora_net::chat_service::{HistoryRow, TimeUuidKey, format_time_uuid};
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::time::Instant;
use uuid::Uuid;

/// A runaway loop guard: no real sequence of turns comes near it.
const MAX_STEPS_PER_POLL: usize = 100_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum StateSafety {
    #[default]
    Unknown,
    Safe,
    Unsafe,
}

/// A resident's name lookup (Metadata).
#[derive(Debug, Clone, Default)]
pub(super) enum MetaState {
    #[default]
    Unrequested,
    /// One subscription until `deadline`; `subscribed` false once the
    /// delete disconnected it, so a late answer is dropped.
    Pending {
        deadline: Instant,
        subscribed: bool,
    },
    Resolved(AvatarName),
    /// Timed out: dormant until the next list, priority or context change.
    Failed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Discovery {
    #[default]
    None,
    /// An IM came from a resident the last list did not have.
    Pending,
    /// The next list did not have it either: one more list after 5 s.
    Confirming,
}

/// One resident's scheduling record (Resident).
#[derive(Debug, Clone)]
pub(super) struct Resident {
    /// The list's last message id; compared for equality only.
    pub(super) advertised_token: String,
    /// The advertised id a complete pass covered.
    pub(super) covered_token: String,
    pub(super) archive_serial: u32,
    /// `Failed` until a scan of this login described the file.
    pub(super) summary: ArchiveScan,
    pub(super) metadata: MetaState,
    pub(super) listed: bool,
    pub(super) forced_followup: bool,
    pub(super) first_request_started: bool,
    pub(super) retry_used: bool,
    pub(super) metadata_waiting: bool,
    /// The quiet deadline of the resident's IM burst.
    pub(super) activity_refresh_due: Option<Instant>,
    pub(super) activity_discovery: Discovery,
    pub(super) work_active: bool,
    pub(super) head_preview: Vec<HistoryRow>,
}

impl Resident {
    fn new() -> Resident {
        Resident {
            advertised_token: String::new(),
            covered_token: String::new(),
            archive_serial: 0,
            summary: ArchiveScan {
                state: ArchiveState::Failed,
                ..ArchiveScan::default()
            },
            metadata: MetaState::Unrequested,
            listed: false,
            forced_followup: false,
            first_request_started: false,
            retry_used: false,
            metadata_waiting: false,
            activity_refresh_due: None,
            activity_discovery: Discovery::None,
            work_active: false,
            head_preview: Vec::new(),
        }
    }

    pub(super) fn has_archive_rows(&self) -> bool {
        self.summary.oldest.is_some()
    }

    pub(super) fn name(&self) -> Option<&AvatarName> {
        match &self.metadata {
            MetaState::Resolved(name) => Some(name),
            _ => None,
        }
    }
}

/// Where a manager turn starts again after an await: the coroutine went on
/// with the rest of its turn after the state load, a failed preparation and
/// a failed index write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Phase {
    Top,
    AfterState,
    AfterPrepare,
    AfterIndex,
}

/// Where the coroutine stands: a turn to run, a wait, or an await of the
/// reply with a ticket.
#[derive(Debug)]
pub(super) enum Step {
    Turn(Phase),
    /// waitForWake(nearestWait()): until a wake input or the deadline.
    Sleeping {
        until: Instant,
    },
    LoadingState(u64),
    ListingArchives(u64),
    PreparingOne {
        ticket: u64,
        resident: Uuid,
    },
    UpdatingIndex(u64),
    /// pace() of requestList.
    ListPacing {
        ctx: Context,
    },
    ListInFlight {
        ticket: u64,
        ctx: Context,
    },
    Pass(Box<Pass>, PassWait),
    DeletePending(u64),
    DeleteSweep {
        ticket: u64,
        boundary: u64,
    },
    DeleteFinal {
        ticket: u64,
    },
}

impl Default for Step {
    fn default() -> Self {
        Step::Turn(Phase::Top)
    }
}

/// What a resident's pass is suspended on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PassWait {
    Pacing,
    Response(u64),
    Prepare(u64),
    Publish(u64),
}

impl Step {
    /// The ticket this step awaits, with the kind of reply it takes.
    fn awaited(&self) -> Option<(u64, ReplyKind)> {
        Some(match self {
            Step::LoadingState(t) => (*t, ReplyKind::State),
            Step::ListingArchives(t) => (*t, ReplyKind::Archives),
            Step::PreparingOne { ticket, .. } => (*ticket, ReplyKind::Prepared),
            Step::UpdatingIndex(t) => (*t, ReplyKind::Index),
            Step::ListInFlight { ticket, .. } => (*ticket, ReplyKind::Http),
            Step::Pass(_, PassWait::Response(t)) => (*t, ReplyKind::Http),
            Step::Pass(_, PassWait::Prepare(t)) => (*t, ReplyKind::Prepared),
            Step::Pass(_, PassWait::Publish(t)) => (*t, ReplyKind::Published),
            Step::DeletePending(t) => (*t, ReplyKind::DeleteState),
            Step::DeleteSweep { ticket, .. } => (*ticket, ReplyKind::Swept),
            Step::DeleteFinal { ticket } => (*ticket, ReplyKind::StateWritten),
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplyKind {
    Http,
    State,
    Archives,
    Prepared,
    Published,
    Index,
    DeleteState,
    Swept,
    StateWritten,
}

/// The answer to an awaited action, kept until the next poll resumes.
#[derive(Debug)]
pub(super) enum Reply {
    Http { status: u16, body: Option<Llsd> },
    State(StateLoad),
    Archives { residents: Vec<Uuid>, artifacts_exist: bool },
    Prepared(Option<PreparedArchive>),
    Published(Option<ArchiveScan>),
    Index(bool),
    DeleteState(Option<u64>),
    Swept(bool),
    StateWritten(bool),
}

impl Reply {
    fn kind(&self) -> ReplyKind {
        match self {
            Reply::Http { .. } => ReplyKind::Http,
            Reply::State(_) => ReplyKind::State,
            Reply::Archives { .. } => ReplyKind::Archives,
            Reply::Prepared(_) => ReplyKind::Prepared,
            Reply::Published(_) => ReplyKind::Published,
            Reply::Index(_) => ReplyKind::Index,
            Reply::DeleteState(_) => ReplyKind::DeleteState,
            Reply::Swept(_) => ReplyKind::Swept,
            Reply::StateWritten(_) => ReplyKind::StateWritten,
        }
    }
}

/// Whether the machine runs on within the same poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Flow {
    Continue,
    Wait,
}

/// One login's state (Runtime); reset by `stop`.
#[derive(Debug, Default)]
pub(super) struct Runtime {
    pub(super) running: bool,
    pub(super) rollout: bool,
    /// wake_pending: an input asked the sleeping manager to run a turn.
    pub(super) woken: bool,
    pub(super) initialized_archives: bool,
    pub(super) list_needed: bool,
    pub(super) list_retry_used: bool,
    pub(super) delete_requested: bool,
    pub(super) delete_active: bool,
    pub(super) index_dirty: bool,
    pub(super) local_content_exists: bool,
    pub(super) delete_click_ticks: u64,
    pub(super) deleted_before_ticks: u64,
    pub(super) state_safety: StateSafety,
    pub(super) cleanup_pending: bool,
    pub(super) network_not_before: Option<Instant>,
    pub(super) next_list: Option<Instant>,
    /// The context the last turn sampled.
    pub(super) context: Context,
    pub(super) agent_id: Uuid,
    pub(super) active_resident: Option<Uuid>,
    pub(super) priority_resident: Option<Uuid>,
    pub(super) queue: VecDeque<Uuid>,
    /// Ordered by id, as the LL viewer's std::map: the order of the
    /// maintenance turns and of the index.
    pub(super) residents: BTreeMap<Uuid, Resident>,
    pub(super) step: Step,
    pub(super) reply: Option<Reply>,
    pub(super) outbox: Vec<Action>,
    pub(super) blocked_logged: bool,
}

/// The Chat Service history manager of one viewer: one login at a time.
/// Feed it the inputs, call [`Manager::poll`] after them and at
/// [`Manager::next_wake`], and carry out the actions in order.
#[derive(Debug, Default)]
pub struct Manager {
    pub(super) epoch: u32,
    pub(super) next_ticket: u64,
    pub(super) gates: Gates,
    /// The region's context now (sampleContext); the turns compare it with
    /// the one they sampled.
    pub(super) live: Context,
    pub(super) rt: Runtime,
}

impl Manager {
    pub fn new() -> Manager {
        Manager::default()
    }

    // ---- lifecycle and inputs ------------------------------------------

    /// Login complete. `rollout` is ChatServiceEnabled, read once per
    /// login. Port of LLChatServiceHistory::start (with regionChanged's
    /// wake).
    pub fn start(&mut self, agent_id: Uuid, rollout: bool) {
        self.stop();
        self.epoch = self.epoch.wrapping_add(1);
        self.rt.running = true;
        self.rt.rollout = rollout;
        self.rt.agent_id = agent_id;
        self.rt.state_safety = StateSafety::Unknown;
        self.rt.next_list = None;
        self.rt.list_needed = true;
        self.rt.woken = true;
    }

    /// Logout or disconnect; idempotent. Every awaited reply of the login
    /// is dropped from now on. Port of LLChatServiceHistory::stop.
    pub fn stop(&mut self) {
        if self.rt.running {
            self.rt.running = false;
            self.epoch = self.epoch.wrapping_add(1);
        }
        self.rt = Runtime::default();
    }

    /// The agent's region and its two capabilities, `None` without a
    /// region or before its capabilities (sampleContext). Port of
    /// LLChatServiceHistory::regionChanged: the next turn compares it.
    pub fn set_context(&mut self, context: Option<Context>) {
        self.live = context.unwrap_or_default();
        self.wake();
    }

    /// The consent setting and the mute list's source. A consent change
    /// runs the KeepConversationLogTranscripts observer of start().
    pub fn set_gates(&mut self, gates: Gates) {
        let old = self.gates;
        self.gates = gates;
        if !self.rt.running {
            return;
        }
        if old.consent != gates.consent {
            // Consent lost revokes the transient presentation at once.
            let allowed = gates.consent;
            let ids: Vec<Uuid> = self.rt.residents.keys().copied().collect();
            for id in ids {
                if let Some(r) = self.rt.residents.get_mut(&id)
                    && !allowed
                {
                    r.activity_refresh_due = None;
                    r.head_preview.clear();
                    r.work_active = false;
                }
                self.publish_snapshot(id);
            }
            self.wake();
        }
        if old.mute_list_server_loaded != gates.mute_list_server_loaded {
            // The LL viewer does not wake here and waits for an unrelated
            // wake (up to the hourly list); waking only starts sooner.
            self.wake();
        }
    }

    /// LogShowHistory changed (the observer of start()).
    pub fn show_history_changed(&mut self) {
        if self.rt.running {
            self.rt.outbox.push(Action::StyleUpdate);
        }
    }

    /// A direct session was created, or a conversation preview opened on
    /// it. Port of LLChatServiceHistory::prioritizeResident.
    pub fn prioritize(&mut self, id: Uuid, follow_active_request: bool, env: &dyn Env) {
        if !self.rt.running || id.is_nil() {
            return;
        }
        let active = self.rt.active_resident == Some(id);
        let r = self.resident_mut(id);
        r.retry_used = false;
        if matches!(r.metadata, MetaState::Failed) {
            r.metadata = MetaState::Unrequested;
        }
        if active {
            // A trigger after the active request began records one
            // follow-up pass.
            if follow_active_request && r.first_request_started {
                r.forced_followup = true;
            }
        } else {
            self.queue_resident(id, true);
            self.rt.priority_resident = Some(id);
        }
        if !self.resident_mut(id).listed {
            self.rt.list_needed = true;
            self.rt.list_retry_used = false;
        }
        let potentially_active = self.service_presentation_allowed() && self.live.complete() && !self.uuid_blocked(id, env);
        self.set_work_active(id, potentially_active);
        self.wake();
    }

    /// An incoming direct IM that is not a system notice, online or
    /// offline, or an outgoing IM_NOTHING_SPECIAL once sent. Port of
    /// LLChatServiceHistory::noteDirectMessageActivity.
    pub fn note_activity(&mut self, id: Uuid, now: Instant, env: &dyn Env) {
        if !self.rt.running || !self.rt.rollout || !self.gates.consent || self.rt.delete_requested || id.is_nil() || id == self.rt.agent_id
        {
            return;
        }
        // Every IM resets the quiet deadline; a first contact also starts
        // the bounded discovery at once.
        let r = self.resident_mut(id);
        r.activity_refresh_due = Some(now + ACTIVITY_REFRESH_DELAY);
        if !r.listed && r.activity_discovery == Discovery::None {
            r.activity_discovery = Discovery::Pending;
            self.prioritize(id, false, env);
        } else {
            self.wake();
        }
    }

    /// The answer of an [`Action::ResolveName`]; `None` (no name came) is
    /// left to the lookup's deadline. Port of the callback of
    /// ensureMetadata.
    pub fn name_resolved(&mut self, id: Uuid, name: Option<AvatarName>) {
        let Some(name) = name else { return };
        if !self.rt.running {
            return;
        }
        let priority = self.rt.priority_resident == Some(id);
        let Some(r) = self.rt.residents.get_mut(&id) else { return };
        if !matches!(r.metadata, MetaState::Pending { subscribed: true, .. }) {
            return;
        }
        r.metadata = MetaState::Resolved(name);
        let waiting = std::mem::take(&mut r.metadata_waiting);
        self.rt.index_dirty = true;
        if waiting {
            self.queue_resident(id, priority);
        }
        self.publish_snapshot(id);
        self.wake();
    }

    /// The answer of an [`Action::Http`] (`NetEvent::ChatServiceResponse`):
    /// `status` 0 for a transport failure, 499 for an unreadable body.
    pub fn on_response(&mut self, id: u64, status: u16, body: Option<Llsd>) {
        self.deliver(id, Reply::Http { status, body });
    }

    pub fn state_loaded(&mut self, ticket: u64, state: StateLoad) {
        self.deliver(ticket, Reply::State(state));
    }

    /// The canonical archives found, and whether any other deletable
    /// `chat_service_*` artifact exists.
    pub fn archives_listed(&mut self, ticket: u64, residents: Vec<Uuid>, artifacts_exist: bool) {
        self.deliver(
            ticket,
            Reply::Archives {
                residents,
                artifacts_exist,
            },
        );
    }

    /// `None`: the preparation failed, the archive stays to be prepared.
    pub fn archive_prepared(&mut self, ticket: u64, prepared: Option<PreparedArchive>) {
        self.deliver(ticket, Reply::Prepared(prepared));
    }

    /// The archive's summary after the write; `None`: the write failed or
    /// the file changed behind the manager's back.
    pub fn rows_published(&mut self, ticket: u64, summary: Option<ArchiveScan>) {
        self.deliver(ticket, Reply::Published(summary));
    }

    pub fn index_updated(&mut self, ticket: u64, ok: bool) {
        self.deliver(ticket, Reply::Index(ok));
    }

    /// The boundary the pending state was written with, `None` on failure.
    pub fn delete_state_written(&mut self, ticket: u64, boundary: Option<u64>) {
        self.deliver(ticket, Reply::DeleteState(boundary));
    }

    pub fn transcripts_swept(&mut self, ticket: u64, ok: bool) {
        self.deliver(ticket, Reply::Swept(ok));
    }

    pub fn state_written(&mut self, ticket: u64, ok: bool) {
        self.deliver(ticket, Reply::StateWritten(ok));
    }

    /// A stitched read found a damaged archive (`maintenance_needed`):
    /// unless the archive changed since the read began, rescan it. Port of
    /// the completion of loadStitchedHistory.
    pub fn view_read_damaged(&mut self, epoch: u32, id: Uuid, archive_serial: u32) {
        if epoch != self.epoch {
            return;
        }
        if let Some(r) = self.rt.residents.get_mut(&id)
            && r.archive_serial == archive_serial
        {
            r.summary.state = ArchiveState::Failed;
            r.covered_token.clear();
            self.wake();
        }
    }

    /// Start a stitched read of `id`: the inputs to capture, `None` while
    /// history is suppressed. Asks for the name when service rows are
    /// included. Port of the capture of loadStitchedHistory.
    pub fn begin_view_read(&mut self, id: Uuid, now: Instant, env: &dyn Env) -> Option<ViewRead> {
        if id.is_nil() || self.history_suppressed() {
            return None;
        }
        self.resident_mut(id);
        let include_service = self.service_presentation_allowed();
        if include_service {
            self.ensure_metadata(id, now, env);
        }
        let r = self.rt.residents.get(&id)?;
        Some(ViewRead {
            epoch: self.epoch,
            include_service,
            archive_serial: r.archive_serial,
            deleted_before_ticks: self.rt.deleted_before_ticks,
            preview: if include_service { r.head_preview.clone() } else { Vec::new() },
        })
    }

    // ---- output ---------------------------------------------------------

    /// Run the manager until it waits, and return its side effects in
    /// order. Port of the `manager` coroutine (CS:1942-2090): one unit of
    /// work per turn, the next turn at once while a turn did work.
    pub fn poll(&mut self, now: Instant, env: &dyn Env) -> Vec<Action> {
        let mut steps = 0;
        while self.rt.running {
            let step = std::mem::take(&mut self.rt.step);
            if self.advance(step, now, env) == Flow::Wait {
                break;
            }
            steps += 1;
            if steps >= MAX_STEPS_PER_POLL {
                log::warn!("Chat Service history: the manager did not settle within one poll");
                break;
            }
        }
        self.drain()
    }

    /// When to poll again without a new input; `None` while a reply is
    /// awaited or when stopped. Port of nearestWait, and of pace()'s wait.
    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        if !self.rt.running {
            return None;
        }
        if !self.rt.outbox.is_empty() || self.rt.reply.is_some() {
            return Some(now);
        }
        match &self.rt.step {
            Step::Turn(_) => Some(now),
            Step::Sleeping { until } => Some(if self.rt.woken { now } else { *until }),
            Step::ListPacing { .. } | Step::Pass(_, PassWait::Pacing) => Some(self.pace_deadline(now)),
            _ => None,
        }
    }

    // ---- queries --------------------------------------------------------

    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    pub fn is_running(&self) -> bool {
        self.rt.running
    }

    /// All historical sources are hidden (historySuppressed).
    pub fn history_suppressed(&self) -> bool {
        self.rt.state_safety != StateSafety::Safe || self.rt.cleanup_pending || self.rt.delete_requested
    }

    /// Service rows may be shown (servicePresentationAllowed).
    pub fn service_presentation_allowed(&self) -> bool {
        self.rt.rollout && self.gates.consent && !self.history_suppressed()
    }

    /// The Delete Transcripts button's half of the check
    /// (localHistoryExists()).
    pub fn local_history_exists(&self) -> bool {
        if self.rt.delete_active {
            return false;
        }
        if self.rt.delete_requested || self.rt.cleanup_pending || self.rt.state_safety == StateSafety::Unsafe {
            return true;
        }
        self.rt.residents.values().any(Resident::has_archive_rows) || self.rt.local_content_exists
    }

    /// The resident has archived rows to show (localHistoryExists(id)); a
    /// miss on an unprepared archive wakes the manager.
    pub fn local_history_exists_for(&mut self, id: Uuid) -> bool {
        if !self.service_presentation_allowed() {
            return false;
        }
        match self.rt.residents.get(&id) {
            Some(r) if r.summary.state != ArchiveState::Failed => r.has_archive_rows(),
            _ => {
                self.wake();
                false
            }
        }
    }

    /// The view state of `id` (getSnapshot), current at the time of the
    /// call.
    pub fn snapshot(&self, id: Uuid) -> Snapshot {
        let allowed = self.service_presentation_allowed();
        match self.rt.residents.get(&id) {
            Some(r) => Snapshot {
                service_work_active: r.work_active,
                service_presentation_allowed: allowed,
                archive_serial: r.archive_serial,
                name: r.name().cloned(),
                head_preview: r.head_preview.clone(),
            },
            None => Snapshot {
                service_presentation_allowed: allowed,
                ..Snapshot::default()
            },
        }
    }

    pub fn head_preview(&self, id: Uuid) -> &[HistoryRow] {
        self.rt.residents.get(&id).map_or(&[], |r| r.head_preview.as_slice())
    }

    /// The newest archived key, when the archive has rows.
    pub fn archive_newest(&self, id: Uuid) -> Option<TimeUuidKey> {
        self.rt.residents.get(&id).and_then(|r| r.summary.oldest.and(r.summary.newest))
    }

    pub fn archive_serial(&self, id: Uuid) -> u32 {
        self.rt.residents.get(&id).map_or(0, |r| r.archive_serial)
    }

    pub fn work_active(&self, id: Uuid) -> bool {
        self.rt.residents.get(&id).is_some_and(|r| r.work_active)
    }

    pub fn resolved_name(&self, id: Uuid) -> Option<&AvatarName> {
        self.rt.residents.get(&id).and_then(Resident::name)
    }

    /// `chat_service_after_msg_id` of an outgoing IM, captured before it is
    /// sent: the newest service key the view (`view_latest`, the default
    /// key for none), the archive or the head preview knows; "" for none.
    /// Port of History::outgoingContext (CS:2727-2758).
    pub fn outgoing_after_msg_id(&self, id: Uuid, view_latest: TimeUuidKey) -> String {
        let mut latest = view_latest;
        let none = TimeUuidKey::default();
        if let Some(r) = self.rt.residents.get(&id) {
            if let (Some(_), Some(newest)) = (r.summary.oldest, r.summary.newest)
                && (latest == none || latest < newest)
            {
                latest = newest;
            }
            for row in &r.head_preview {
                if latest == none || latest < row.key {
                    latest = row.key;
                }
            }
        }
        format_time_uuid(&latest)
    }

    // ---- shared helpers -------------------------------------------------

    pub(super) fn ticket(&mut self) -> u64 {
        self.next_ticket += 1;
        self.next_ticket
    }

    /// wakeManager.
    pub(super) fn wake(&mut self) {
        if self.rt.running {
            self.rt.woken = true;
        }
    }

    fn deliver(&mut self, ticket: u64, reply: Reply) {
        if !self.rt.running || self.rt.reply.is_some() {
            return;
        }
        if self.rt.step.awaited() == Some((ticket, reply.kind())) {
            self.rt.reply = Some(reply);
        }
    }

    /// The deliveries the LL viewer queued for the main loop read the
    /// current snapshot when they ran: one per resident is enough.
    fn drain(&mut self) -> Vec<Action> {
        let mut seen = HashSet::new();
        std::mem::take(&mut self.rt.outbox)
            .into_iter()
            .filter(|a| match a {
                Action::SnapshotChanged(id) => seen.insert(*id),
                _ => true,
            })
            .collect()
    }

    pub(super) fn resident_mut(&mut self, id: Uuid) -> &mut Resident {
        self.rt.residents.entry(id).or_insert_with(Resident::new)
    }

    /// publishSnapshot.
    pub(super) fn publish_snapshot(&mut self, id: Uuid) {
        self.rt.outbox.push(Action::SnapshotChanged(id));
    }

    /// setWorkActive.
    pub(super) fn set_work_active(&mut self, id: Uuid, active: bool) {
        let r = self.resident_mut(id);
        if r.work_active != active {
            r.work_active = active;
            self.publish_snapshot(id);
        }
    }

    /// queueResident: one occurrence, at the front for a priority.
    pub(super) fn queue_resident(&mut self, id: Uuid, priority: bool) {
        self.rt.queue.retain(|q| *q != id);
        if priority {
            self.rt.queue.push_front(id);
        } else {
            self.rt.queue.push_back(id);
        }
    }

    pub(super) fn queued(&self, id: Uuid) -> bool {
        self.rt.queue.contains(&id)
    }

    pub(super) fn clear_priority(&mut self, id: Uuid) {
        if self.rt.priority_resident == Some(id) {
            self.rt.priority_resident = None;
        }
    }

    /// Move the account-wide request deadline later, never sooner.
    pub(super) fn hold_network_until(&mut self, until: Instant) {
        self.rt.network_not_before = Some(self.rt.network_not_before.map_or(until, |n| n.max(until)));
    }

    /// uuidBlocked: fail closed until the mute list is the server's.
    pub(super) fn uuid_blocked(&self, id: Uuid, env: &dyn Env) -> bool {
        !self.gates.mute_list_server_loaded || env.is_muted(id)
    }

    /// nameBlocked: also blocked until the name is known to check the
    /// legacy-name mutes.
    pub(super) fn name_blocked(&self, id: Uuid, env: &dyn Env) -> bool {
        if self.uuid_blocked(id, env) {
            return true;
        }
        match self.rt.residents.get(&id).and_then(Resident::name) {
            Some(name) => env.is_muted_name(id, &name.user_name()),
            None => true,
        }
    }

    /// baseNetworkEligible.
    pub(super) fn base_network_eligible(&self, ctx: &Context) -> bool {
        self.rt.running && self.service_presentation_allowed() && ctx.complete() && self.gates.mute_list_server_loaded
    }

    /// networkEligible.
    pub(super) fn network_eligible(&self, id: Uuid, ctx: &Context, env: &dyn Env) -> bool {
        self.base_network_eligible(ctx) && !id.is_nil() && !self.name_blocked(id, env)
    }

    /// Port of ensureMetadata (CS:1013-1068): true once the name is known.
    pub(super) fn ensure_metadata(&mut self, id: Uuid, now: Instant, env: &dyn Env) -> bool {
        if matches!(self.resident_mut(id).metadata, MetaState::Resolved(_)) {
            return true;
        }
        if let Some(name) = env.cached_name(id) {
            self.resident_mut(id).metadata = MetaState::Resolved(name);
            self.rt.index_dirty = true;
            self.publish_snapshot(id);
            return true;
        }
        let r = self.resident_mut(id);
        if matches!(r.metadata, MetaState::Pending { .. } | MetaState::Failed) {
            return false;
        }
        r.metadata = MetaState::Pending {
            deadline: now + NAME_TIMEOUT,
            subscribed: true,
        };
        self.rt.outbox.push(Action::ResolveName(id));
        false
    }

    /// Port of expireMetadata (CS:1854-1870): a timed-out lookup leaves the
    /// resident dormant until a later ordinary trigger.
    pub(super) fn expire_metadata(&mut self, now: Instant) {
        let expired: Vec<Uuid> = self
            .rt
            .residents
            .iter()
            .filter(|(_, r)| matches!(r.metadata, MetaState::Pending { deadline, .. } if deadline <= now))
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            let r = self.resident_mut(id);
            r.metadata = MetaState::Failed;
            r.metadata_waiting = false;
            self.set_work_active(id, false);
            self.clear_priority(id);
        }
    }

    /// Port of activateDueActivityRefreshes (CS:1872-1915).
    pub(super) fn activate_due_activity_refreshes(&mut self, now: Instant, env: &dyn Env) {
        if !self.service_presentation_allowed() {
            for r in self.rt.residents.values_mut() {
                r.activity_refresh_due = None;
            }
            return;
        }
        let due: Vec<Uuid> = self
            .rt
            .residents
            .iter()
            .filter(|(_, r)| r.activity_refresh_due.is_some_and(|d| d <= now))
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            // Consumed before the priority path, so that an expired value
            // cannot spin the manager.
            let r = self.resident_mut(id);
            r.activity_refresh_due = None;
            let name = r.name().map(AvatarName::user_name);
            let (listed, discovery) = (r.listed, r.activity_discovery);
            if self.gates.mute_list_server_loaded && (env.is_muted(id) || name.is_some_and(|n| env.is_muted_name(id, &n))) {
                continue;
            }
            if listed {
                self.prioritize(id, true, env);
            } else if discovery == Discovery::None {
                self.resident_mut(id).activity_discovery = Discovery::Pending;
                self.prioritize(id, false, env);
            }
        }
    }

    /// The end of pace()'s wait: the request deadline, or sooner a name or
    /// quiet deadline.
    pub(super) fn pace_deadline(&self, now: Instant) -> Instant {
        let Some(not_before) = self.rt.network_not_before.filter(|t| *t > now) else {
            return now;
        };
        let mut until = not_before;
        for r in self.rt.residents.values() {
            if let MetaState::Pending { deadline, .. } = r.metadata {
                until = until.min(deadline.max(now + super::MIN_WAIT));
            }
            if let Some(due) = r.activity_refresh_due.filter(|d| *d > now) {
                until = until.min(due);
            }
        }
        until
    }

    /// Port of nearestWait (CS:1917-1940).
    pub(super) fn nearest_wait(&self, now: Instant) -> Instant {
        let mut deadline = match self.rt.next_list {
            Some(t) if t > now => t,
            _ => now + LIST_INTERVAL,
        };
        if let Some(t) = self.rt.network_not_before.filter(|t| *t > now) {
            deadline = deadline.min(t);
        }
        let allowed = self.service_presentation_allowed();
        for r in self.rt.residents.values() {
            if let MetaState::Pending { deadline: d, .. } = r.metadata {
                deadline = deadline.min(d);
            }
            if allowed && let Some(d) = r.activity_refresh_due {
                deadline = deadline.min(d);
            }
        }
        deadline.max(now + MIN_WAIT)
    }

    pub(super) fn continue_at(&mut self, phase: Phase) -> Flow {
        self.rt.step = Step::Turn(phase);
        Flow::Continue
    }

    // ---- the manager loop -----------------------------------------------

    fn advance(&mut self, step: Step, now: Instant, env: &dyn Env) -> Flow {
        match step {
            Step::Turn(phase) => self.turn(phase, now, env),
            Step::Sleeping { until } => {
                if self.rt.woken || now >= until {
                    self.turn(Phase::Top, now, env)
                } else {
                    self.rt.step = Step::Sleeping { until };
                    Flow::Wait
                }
            }
            // pace() expires the name lookups after each of its waits.
            Step::ListPacing { ctx } => {
                self.expire_metadata(now);
                self.list_paced(ctx, now, env)
            }
            Step::Pass(pass, PassWait::Pacing) => {
                self.expire_metadata(now);
                self.pass_paced(pass, now, env)
            }
            step => match self.rt.reply.take() {
                Some(reply) => self.resume(step, reply, now, env),
                None => {
                    self.rt.step = step;
                    Flow::Wait
                }
            },
        }
    }

    fn resume(&mut self, step: Step, reply: Reply, now: Instant, env: &dyn Env) -> Flow {
        match (step, reply) {
            (Step::LoadingState(_), Reply::State(state)) => self.state_ready(state),
            (
                Step::ListingArchives(_),
                Reply::Archives {
                    residents,
                    artifacts_exist,
                },
            ) => {
                self.rt.local_content_exists = artifacts_exist;
                for id in residents {
                    self.resident_mut(id).summary.state = ArchiveState::Failed;
                }
                self.rt.initialized_archives = true;
                self.rt.index_dirty = true;
                self.continue_at(Phase::Top)
            }
            (Step::PreparingOne { resident, .. }, Reply::Prepared(prepared)) => self.one_archive_prepared(resident, prepared, now, env),
            (Step::UpdatingIndex(_), Reply::Index(ok)) => {
                self.rt.index_dirty |= !ok;
                self.continue_at(if ok { Phase::Top } else { Phase::AfterIndex })
            }
            (Step::ListInFlight { ctx, .. }, Reply::Http { status, body }) => self.list_answered(ctx, status, body, now, env),
            (Step::Pass(pass, PassWait::Response(_)), Reply::Http { status, body }) => self.pass_answered(pass, status, body, now, env),
            (Step::Pass(pass, PassWait::Prepare(_)), Reply::Prepared(prepared)) => self.pass_prepared(pass, prepared, now, env),
            (Step::Pass(pass, PassWait::Publish(_)), Reply::Published(summary)) => self.pass_published(pass, summary, env),
            (Step::DeletePending(_), Reply::DeleteState(boundary)) => self.delete_pending_written(boundary),
            (Step::DeleteSweep { boundary, .. }, Reply::Swept(ok)) => self.delete_swept(boundary, ok),
            (Step::DeleteFinal { .. }, Reply::StateWritten(ok)) => self.delete_final_written(ok),
            // `deliver` only keeps the kind the step awaits.
            (step, _) => {
                self.rt.step = step;
                Flow::Wait
            }
        }
    }

    /// One manager turn (CS:1948-2089), from `phase` on.
    fn turn(&mut self, phase: Phase, now: Instant, env: &dyn Env) -> Flow {
        if phase == Phase::Top {
            if self.rt.delete_active {
                return self.run_delete();
            }
            if self.rt.state_safety == StateSafety::Unknown {
                // Privacy state comes before archives, views or network.
                let ticket = self.ticket();
                self.rt.outbox.push(Action::LoadState { ticket });
                self.rt.step = Step::LoadingState(ticket);
                return Flow::Wait;
            }
        }
        if phase <= Phase::AfterState {
            let safe = self.rt.state_safety == StateSafety::Safe;
            if safe && !self.rt.initialized_archives {
                let ticket = self.ticket();
                self.rt.outbox.push(Action::ListArchives { ticket });
                self.rt.step = Step::ListingArchives(ticket);
                return Flow::Wait;
            }

            self.expire_metadata(now);
            self.activate_due_activity_refreshes(now, env);

            let ctx = self.live.clone();
            if ctx != self.rt.context {
                // A new context restarts the discovery and keeps the
                // archives' coverage and the priorities.
                self.rt.context = ctx.clone();
                if ctx.complete() {
                    self.rt.list_needed = true;
                    self.rt.list_retry_used = false;
                    for r in self.rt.residents.values_mut() {
                        if matches!(r.metadata, MetaState::Failed) {
                            r.metadata = MetaState::Unrequested;
                        }
                    }
                } else {
                    self.all_inactive();
                }
            }

            let base_network = self.base_network_eligible(&ctx);
            if !base_network && ctx.complete() && !self.rt.blocked_logged {
                self.rt.blocked_logged = true;
                log::info!(
                    "Chat Service history: network sync blocked: rollout={}, consent={}, state_safe={}, cleanup_pending={}, delete_requested={}, mute_authoritative={}",
                    self.rt.rollout,
                    self.gates.consent,
                    self.rt.state_safety == StateSafety::Safe,
                    self.rt.cleanup_pending,
                    self.rt.delete_requested,
                    self.gates.mute_list_server_loaded
                );
            }
            if base_network {
                if self.rt.next_list.is_none_or(|t| now >= t) {
                    self.rt.list_needed = true;
                    self.rt.list_retry_used = false;
                }
                if self.rt.list_needed {
                    return self.request_list(ctx, now, env);
                }
                if let Some(id) = self.rt.queue.pop_front() {
                    return self.sync_resident(id, ctx, now, env);
                }
                // Existing archives use the same names as the views.
                let unnamed: Vec<Uuid> = self
                    .rt
                    .residents
                    .iter()
                    .filter(|(_, r)| r.has_archive_rows() && matches!(r.metadata, MetaState::Unrequested))
                    .map(|(id, _)| *id)
                    .collect();
                for id in unnamed {
                    self.ensure_metadata(id, now, env);
                }
            } else {
                self.rt.active_resident = None;
                self.all_inactive();
            }

            // Storage maintenance yields to all network work: one archive
            // per turn (prepareOneArchive).
            if self.rt.state_safety == StateSafety::Safe && !self.rt.cleanup_pending {
                let failed = self
                    .rt
                    .residents
                    .iter()
                    .find(|(_, r)| r.summary.state == ArchiveState::Failed)
                    .map(|(id, _)| *id);
                if let Some(resident) = failed {
                    let ticket = self.ticket();
                    self.rt.outbox.push(Action::PrepareArchive {
                        ticket,
                        resident,
                        deleted_before_ticks: self.rt.deleted_before_ticks,
                    });
                    self.rt.step = Step::PreparingOne { ticket, resident };
                    return Flow::Wait;
                }
            }
        }
        if phase <= Phase::AfterPrepare && self.rt.index_dirty && self.rt.state_safety == StateSafety::Safe && !self.rt.cleanup_pending {
            return self.update_index();
        }
        self.rt.woken = false;
        self.rt.step = Step::Sleeping {
            until: self.nearest_wait(now),
        };
        Flow::Wait
    }

    fn all_inactive(&mut self) {
        let ids: Vec<Uuid> = self.rt.residents.keys().copied().collect();
        for id in ids {
            self.set_work_active(id, false);
        }
    }

    /// The end of the state load of the first turn (CS:1953-1981).
    fn state_ready(&mut self, state: StateLoad) -> Flow {
        let (safety, boundary, pending) = match state {
            StateLoad::Safe {
                deleted_before_ticks,
                cleanup_pending,
            } => (StateSafety::Safe, deleted_before_ticks, cleanup_pending),
            StateLoad::Unsafe => (StateSafety::Unsafe, 0, false),
        };
        self.rt.state_safety = safety;
        self.rt.deleted_before_ticks = boundary;
        self.rt.cleanup_pending = pending;
        self.rt.outbox.push(Action::TranscriptCreated);
        if !self.history_suppressed() {
            self.rt.outbox.push(Action::StyleUpdate);
        }
        if safety == StateSafety::Safe && pending {
            // A delete interrupted by a crash resumes on its own.
            self.rt.delete_active = true;
            self.rt.delete_requested = true;
            self.rt.delete_click_ticks = boundary;
            return self.continue_at(Phase::Top);
        }
        self.continue_at(Phase::AfterState)
    }

    /// The end of prepareOneArchive's preparation (CS:1803-1829).
    fn one_archive_prepared(&mut self, id: Uuid, prepared: Option<PreparedArchive>, now: Instant, env: &dyn Env) -> Flow {
        if !self.apply_prepared(id, prepared) {
            return self.continue_at(Phase::AfterPrepare);
        }
        if self.rt.residents.get(&id).is_some_and(Resident::has_archive_rows) {
            self.ensure_metadata(id, now, env);
        }
        self.rt.outbox.push(Action::TranscriptCreated);
        self.continue_at(Phase::Top)
    }

    /// The end of prepareResidentArchive (CS:1294-1322): false when the
    /// preparation failed.
    pub(super) fn apply_prepared(&mut self, id: Uuid, prepared: Option<PreparedArchive>) -> bool {
        let Some(prepared) = prepared else { return false };
        let mut summary = prepared.summary;
        summary.display_rows.clear();
        let r = self.resident_mut(id);
        r.summary = summary;
        if prepared.bytes_changed {
            // A repair invalidates the views and the coverage.
            r.archive_serial = r.archive_serial.wrapping_add(1);
            r.covered_token.clear();
            self.rt.local_content_exists = true;
        }
        self.rt.index_dirty = true;
        self.publish_snapshot(id);
        true
    }

    /// Port of updateIndex (CS:1831-1852): the dirty latch is cleared
    /// before the write, so a name resolved meanwhile asks for another.
    fn update_index(&mut self) -> Flow {
        self.rt.index_dirty = false;
        let entries = self
            .rt
            .residents
            .iter()
            .filter(|(_, r)| r.summary.state == ArchiveState::Valid)
            .map(|(id, r)| IndexEntry {
                resident: *id,
                name: r.name().cloned(),
            })
            .collect();
        let ticket = self.ticket();
        self.rt.outbox.push(Action::UpdateIndex { ticket, entries });
        self.rt.step = Step::UpdatingIndex(ticket);
        Flow::Wait
    }
}
