//! The network half of the manager: the conversation list and one
//! resident's pass through its history pages. Port of requestList,
//! processList, syncResident, pace and handleRequestFailure
//! (indra/newview/llchatservicehistory.cpp, LL viewer 26.5).

use super::manager::{Discovery, Flow, MetaState, PassWait, Phase, Step};
use super::{Action, Context, Env, LIST_INTERVAL, Manager, PreparedArchive, RATE_LIMIT_DELAY, REQUEST_SPACING, RETRY_DELAY};
use crate::world::chat_log::service::csv::{ArchiveScan, ArchiveState};
use aurora_llsd::Llsd;
use aurora_net::chat_service::{
    HistoryRow, MAX_PASS_BYTES, MAX_PASS_ROWS, Page, TimeUuidKey, history_request_body, validate_conversation_list_checked,
    validate_history_page_checked,
};
use std::collections::HashSet;
use std::time::Instant;
use uuid::Uuid;

/// The locals of one syncResident call, kept across its suspensions.
#[derive(Debug)]
pub(super) struct Pass {
    pub(super) resident: Uuid,
    /// The context of the turn that started the pass: any change ends it.
    pub(super) ctx: Context,
    /// The archive was not scanned yet: done after the first page shows.
    pub(super) needs_prepare: bool,
    /// The archive has rows: paging stops at its newest key.
    pub(super) had_boundary: bool,
    pub(super) stored_newest: Option<TimeUuidKey>,
    /// Rows newer than the archive, newest first.
    pub(super) staged: Vec<HistoryRow>,
    pub(super) staged_bytes: usize,
    pub(super) returned_rows: usize,
    /// `before_msg_id` of the next request, empty for the head page.
    pub(super) cursor: String,
    pub(super) complete: bool,
    pub(super) exposed_preview: bool,
    /// The page held while its archive is prepared.
    pub(super) page: Option<Page>,
    /// How many rows the publication carries (they moved into the action).
    pub(super) published_rows: usize,
}

/// pace()'s answer.
enum Pace {
    Go,
    Wait,
    Abort,
}

/// retryable (CS:989-992): transport failures, 408 and 5xx; never 499.
fn retryable(status: u16) -> bool {
    status < 100 || status == 408 || status >= 500
}

fn success(status: u16) -> bool {
    (200..300).contains(&status)
}

impl Manager {
    /// One pass of pace() (CS:1100-1146): the context and the deletion
    /// latch first, then the resident's eligibility, then the due activity
    /// refreshes, then the account-wide request deadline.
    fn pace(&mut self, ctx: &Context, resident: Option<Uuid>, now: Instant, env: &dyn Env) -> Pace {
        if self.live != *ctx || self.rt.delete_requested {
            return Pace::Abort;
        }
        if let Some(id) = resident
            && (!self.rt.residents.contains_key(&id) || !self.network_eligible(id, ctx, env))
        {
            return Pace::Abort;
        }
        // Quiet expiries during a cooldown take their priority without
        // jumping the cooldown.
        self.activate_due_activity_refreshes(now, env);
        if self.rt.network_not_before.is_none_or(|t| t <= now) {
            Pace::Go
        } else {
            Pace::Wait
        }
    }

    /// request(): the spacing counts from the start of every request.
    fn send(&mut self, now: Instant, url: String, body: Option<Llsd>) -> u64 {
        self.hold_network_until(now + REQUEST_SPACING);
        let id = self.ticket();
        self.rt.outbox.push(Action::Http { id, url, body });
        id
    }

    /// Port of requestList (CS:1234-1292), up to its request.
    pub(super) fn request_list(&mut self, ctx: Context, now: Instant, env: &dyn Env) -> Flow {
        self.list_paced(ctx, now, env)
    }

    pub(super) fn list_paced(&mut self, ctx: Context, now: Instant, env: &dyn Env) -> Flow {
        match self.pace(&ctx, None, now, env) {
            Pace::Wait => {
                self.rt.step = Step::ListPacing { ctx };
                Flow::Wait
            }
            Pace::Abort => self.continue_at(Phase::Top),
            Pace::Go => {
                if !self.base_network_eligible(&ctx) {
                    return self.continue_at(Phase::Top);
                }
                let ticket = self.send(now, ctx.list_url.clone(), None);
                self.rt.step = Step::ListInFlight { ticket, ctx };
                Flow::Wait
            }
        }
    }

    /// The rest of requestList, once its response came.
    pub(super) fn list_answered(&mut self, ctx: Context, status: u16, body: Option<Llsd>, now: Instant, _env: &dyn Env) -> Flow {
        log::info!("Chat Service history: conversation-list response: status={status}");
        if self.live != ctx || self.rt.delete_requested || !self.base_network_eligible(&ctx) {
            return self.continue_at(Phase::Top);
        }
        if status == 429 {
            self.hold_network_until(now + RATE_LIMIT_DELAY);
            self.rt.list_needed = true;
            return self.continue_at(Phase::Top);
        }
        if success(status) && body.as_ref().is_none_or(Llsd::is_undef) {
            log::warn!("Chat Service history: conversation-list refused: empty body");
        }
        if let Some(body) = body.filter(|b| !b.is_undef() && success(status))
            && let Some(confirm) = self.process_list(&body)
        {
            self.rt.list_needed = confirm;
            self.rt.list_retry_used = false;
            self.rt.next_list = Some(now + LIST_INTERVAL);
            if confirm {
                self.hold_network_until(now + RETRY_DELAY);
            }
            return self.continue_at(Phase::Top);
        }
        if retryable(status) && !self.rt.list_retry_used {
            self.rt.list_retry_used = true;
            self.hold_network_until(now + RETRY_DELAY);
            self.rt.list_needed = true;
        } else {
            self.rt.list_needed = false;
            self.rt.next_list = Some(now + LIST_INTERVAL);
        }
        self.continue_at(Phase::Top)
    }

    /// Port of processList (CS:1169-1232): `None` when the list is
    /// rejected (nothing changes), else whether an unknown conversation
    /// needs its confirmation list. Either outcome is logged once, with the
    /// refusal's reason: a refused list silently stops every history pass.
    fn process_list(&mut self, body: &Llsd) -> Option<bool> {
        let list = match validate_conversation_list_checked(body, self.rt.agent_id) {
            Ok(list) => list,
            Err(refusal) => {
                log::warn!("Chat Service history: conversation-list refused: {refusal}");
                return None;
            }
        };
        let entries = list.entries;
        let listed: HashSet<Uuid> = entries.iter().map(|e| e.resident_id).collect();
        let mut confirm = false;

        // The first miss after an activity hint keeps its placeholder for
        // one delayed confirmation; the next miss retires it.
        let known: Vec<Uuid> = self.rt.residents.keys().copied().collect();
        for id in known {
            let r = self.resident_mut(id);
            r.listed = listed.contains(&id);
            if r.listed {
                continue;
            }
            if r.activity_discovery == Discovery::Pending {
                r.activity_discovery = Discovery::Confirming;
                confirm = true;
                continue;
            }
            r.activity_discovery = Discovery::None;
            r.metadata_waiting = false;
            self.rt.queue.retain(|q| *q != id);
            self.clear_priority(id);
            self.set_work_active(id, false);
        }
        let mut queued = 0usize;
        for entry in entries {
            let id = entry.resident_id;
            let placeholder = self.queued(id);
            let r = self.resident_mut(id);
            let changed = r.advertised_token != entry.last_msg_id;
            r.advertised_token = entry.last_msg_id;
            r.listed = true;
            r.activity_discovery = Discovery::None;
            if matches!(r.metadata, MetaState::Failed) {
                r.metadata = MetaState::Unrequested;
            }
            // Empty coverage queues the first discovery; later lists queue
            // the changed or uncovered heads (equality, not an order).
            if !placeholder && (changed || r.covered_token != r.advertised_token) {
                r.retry_used = false;
                self.queue_resident(id, false);
                queued += 1;
            }
        }
        log::info!(
            "Chat Service history: conversation-list accepted: entries={}, direct={}, group={}, adhoc={}, self={}, queued={queued}",
            list.total,
            list.direct,
            list.group,
            list.adhoc,
            list.own
        );
        Some(confirm)
    }

    /// Port of syncResident (CS:1324-1598), up to its first pace().
    pub(super) fn sync_resident(&mut self, id: Uuid, ctx: Context, now: Instant, env: &dyn Env) -> Flow {
        let Some(r) = self.rt.residents.get_mut(&id) else {
            return self.continue_at(Phase::Top);
        };
        if !r.listed {
            r.activity_discovery = Discovery::None;
            r.head_preview.clear();
            self.set_work_active(id, false);
            self.clear_priority(id);
            return self.continue_at(Phase::Top);
        }
        if self.uuid_blocked(id, env) {
            self.set_work_active(id, false);
            self.clear_priority(id);
            return self.continue_at(Phase::Top);
        }
        if !self.ensure_metadata(id, now, env) {
            // Queued again when the name comes.
            self.resident_mut(id).metadata_waiting = true;
            return self.continue_at(Phase::Top);
        }
        self.resident_mut(id).metadata_waiting = false;
        if !self.network_eligible(id, &ctx, env) {
            self.set_work_active(id, false);
            self.clear_priority(id);
            return self.continue_at(Phase::Top);
        }
        self.set_work_active(id, true);
        self.rt.active_resident = Some(id);
        let r = self.resident_mut(id);
        r.first_request_started = false;

        let needs_prepare = r.summary.state == ArchiveState::Failed;
        let pass = Pass {
            resident: id,
            ctx,
            needs_prepare,
            had_boundary: !needs_prepare && r.summary.state == ArchiveState::Valid && r.has_archive_rows(),
            stored_newest: r.summary.newest,
            staged: Vec::new(),
            staged_bytes: 0,
            returned_rows: 0,
            cursor: String::new(),
            complete: false,
            exposed_preview: false,
            page: None,
            published_rows: 0,
        };
        self.pass_paced(Box::new(pass), now, env)
    }

    /// The head of the pass's loop: pace, priority, then the request.
    pub(super) fn pass_paced(&mut self, pass: Box<Pass>, now: Instant, env: &dyn Env) -> Flow {
        let id = pass.resident;
        match self.pace(&pass.ctx, Some(id), now, env) {
            Pace::Wait => {
                self.rt.step = Step::Pass(pass, PassWait::Pacing);
                Flow::Wait
            }
            Pace::Abort => {
                // An account-wide or context gate keeps one queued
                // occurrence; a blocked resident stays dormant until a fresh
                // ordinary trigger.
                self.set_work_active(id, false);
                let temporarily_ineligible = !self.rt.delete_requested
                    && (self.live != pass.ctx || !self.base_network_eligible(&pass.ctx) || !self.gates.mute_list_server_loaded);
                if temporarily_ineligible && !self.queued(id) {
                    let priority = self.rt.priority_resident == Some(id);
                    self.queue_resident(id, priority);
                }
                self.finish_pass(id)
            }
            Pace::Go => {
                if self.preempted(id) {
                    return self.finish_pass(id);
                }
                // Only the head request covers an elapsed quiet deadline;
                // continuation pages never consume a newer one.
                let r = self.resident_mut(id);
                if !r.first_request_started && pass.cursor.is_empty() && r.activity_refresh_due.is_some_and(|d| d <= now) {
                    r.activity_refresh_due = None;
                }
                r.first_request_started = true;
                let body = history_request_body(self.rt.agent_id, id, &pass.cursor);
                let ticket = self.send(now, pass.ctx.history_url.clone(), Some(body));
                self.rt.step = Step::Pass(pass, PassWait::Response(ticket));
                Flow::Wait
            }
        }
    }

    /// Another resident took the priority: this one goes to the back of
    /// the queue and its pass starts again from the head later.
    fn preempted(&mut self, id: Uuid) -> bool {
        if self.rt.priority_resident.is_some_and(|p| p != id) {
            self.queue_resident(id, false);
            return true;
        }
        false
    }

    /// A history page came back.
    pub(super) fn pass_answered(&mut self, mut pass: Box<Pass>, status: u16, body: Option<Llsd>, now: Instant, env: &dyn Env) -> Flow {
        let id = pass.resident;
        log::info!(
            "Chat Service history: history response: resident_id={id}, status={status}, cursor={}",
            if pass.cursor.is_empty() { "head" } else { "continuation" }
        );
        if self.rt.delete_requested || self.live != pass.ctx || !self.network_eligible(id, &pass.ctx, env) {
            return self.finish_pass(id);
        }
        if !success(status) {
            self.handle_request_failure(id, status, now);
            return self.finish_pass(id);
        }
        // A rejected page is not retried.
        let body = body.unwrap_or_default();
        let page = match validate_history_page_checked(&body, self.rt.agent_id, id, &pass.cursor, self.rt.deleted_before_ticks) {
            Ok(page) => page,
            Err(refusal) => {
                log::warn!("Chat Service history: history page refused: resident_id={id}, {refusal}");
                return self.finish_pass(id);
            }
        };

        // Priority changes apply only once the whole page validated.
        self.activate_due_activity_refreshes(now, env);
        if self.preempted(id) {
            return self.finish_pass(id);
        }
        if !pass.exposed_preview && !page.rows.is_empty() {
            self.resident_mut(id).head_preview = page.rows.clone();
            pass.exposed_preview = true;
            self.publish_snapshot(id);
        }

        // The first page shows before a possibly years-long archive scan.
        if pass.needs_prepare {
            pass.page = Some(page);
            let ticket = self.ticket();
            self.rt.outbox.push(Action::PrepareArchive {
                ticket,
                resident: id,
                deleted_before_ticks: self.rt.deleted_before_ticks,
            });
            self.rt.step = Step::Pass(pass, PassWait::Prepare(ticket));
            return Flow::Wait;
        }
        self.pass_rows(pass, page, now, env)
    }

    /// The archive of the pass was prepared.
    pub(super) fn pass_prepared(&mut self, mut pass: Box<Pass>, prepared: Option<PreparedArchive>, now: Instant, env: &dyn Env) -> Flow {
        let id = pass.resident;
        if !self.apply_prepared(id, prepared) {
            return self.finish_pass(id);
        }
        pass.needs_prepare = false;
        let r = self.resident_mut(id);
        pass.had_boundary = r.summary.state == ArchiveState::Valid && r.has_archive_rows();
        pass.stored_newest = r.summary.newest;
        self.activate_due_activity_refreshes(now, env);
        if self.preempted(id) {
            return self.finish_pass(id);
        }
        match pass.page.take() {
            Some(page) => self.pass_rows(pass, page, now, env),
            None => self.finish_pass(id),
        }
    }

    /// Stage a page's rows above the archive, within the pass bounds.
    fn pass_rows(&mut self, mut pass: Box<Pass>, page: Page, now: Instant, env: &dyn Env) -> Flow {
        let id = pass.resident;
        for row in page.rows {
            // Every returned row counts, even past the archive's end; a
            // bound reached drops the whole pass, nothing written.
            pass.returned_rows += 1;
            if pass.returned_rows > MAX_PASS_ROWS {
                return self.finish_pass(id);
            }
            if pass.had_boundary && pass.stored_newest.is_some_and(|newest| row.key <= newest) {
                pass.complete = true;
                break;
            }
            let bytes = row.staged_bytes();
            if pass.staged_bytes + bytes > MAX_PASS_BYTES {
                return self.finish_pass(id);
            }
            pass.staged_bytes += bytes;
            pass.staged.push(row);
        }
        pass.complete |= page.terminal;
        pass.cursor = page.next_cursor;
        if !pass.complete {
            return self.pass_paced(pass, now, env);
        }
        self.pass_complete(pass, env)
    }

    /// The pass reached the archive or the end: publish oldest first.
    fn pass_complete(&mut self, mut pass: Box<Pass>, env: &dyn Env) -> Flow {
        let id = pass.resident;
        if !self.network_eligible(id, &pass.ctx, env) || self.rt.delete_requested || self.live != pass.ctx {
            return self.finish_pass(id);
        }
        if pass.staged.is_empty() {
            return self.pass_end(pass, false, true, env);
        }
        let mut rows = std::mem::take(&mut pass.staged);
        rows.reverse();
        pass.published_rows = rows.len();
        let r = self.resident_mut(id);
        let append = r.summary.state == ArchiveState::Valid && r.has_archive_rows();
        let expected = r.summary.clone();
        let ticket = self.ticket();
        self.rt.outbox.push(Action::PublishRows {
            ticket,
            resident: id,
            rows,
            append,
            expected,
        });
        self.rt.step = Step::Pass(pass, PassWait::Publish(ticket));
        Flow::Wait
    }

    /// The archive write of the pass ended.
    pub(super) fn pass_published(&mut self, pass: Box<Pass>, summary: Option<ArchiveScan>, env: &dyn Env) -> Flow {
        let id = pass.resident;
        let changed = summary.is_some();
        log::info!(
            "Chat Service history: archive publication: resident_id={id}, rows={}, success={changed}",
            pass.published_rows
        );
        // Success and a partial write both invalidate readers and the index.
        let suppressed = self.history_suppressed();
        let r = self.resident_mut(id);
        r.archive_serial = r.archive_serial.wrapping_add(1);
        let mut log_entry = None;
        match summary {
            Some(mut summary) => {
                summary.display_rows.clear();
                r.summary = summary;
                if r.name().is_some() && !suppressed {
                    log_entry = Some(r.summary.newest.and_then(|k| k.unix_seconds()));
                }
            }
            None => {
                // A fresh scan must come before another pass claims coverage.
                r.summary.state = ArchiveState::Failed;
                r.covered_token.clear();
            }
        }
        self.rt.index_dirty = true;
        self.rt.local_content_exists = true;
        if let Some(newest_unix_secs) = log_entry {
            self.rt.outbox.push(Action::ConversationLogEntry {
                resident: id,
                newest_unix_secs,
            });
        }
        self.rt.outbox.push(Action::TranscriptCreated);
        self.pass_end(pass, changed, false, env)
    }

    /// Coverage and the one follow-up pass at the end of a complete pass.
    fn pass_end(&mut self, pass: Box<Pass>, changed: bool, staged_empty: bool, env: &dyn Env) -> Flow {
        let id = pass.resident;
        if self.rt.delete_requested {
            return self.finish_pass(id);
        }
        let eligible = self.network_eligible(id, &pass.ctx, env);
        let r = self.resident_mut(id);
        if changed || staged_empty {
            r.covered_token.clone_from(&r.advertised_token);
            r.retry_used = false;
        }
        if r.forced_followup && eligible {
            r.forced_followup = false;
            self.queue_resident(id, true);
        } else {
            r.work_active = false;
        }
        self.clear_priority(id);
        self.finish_pass(id)
    }

    /// The scope exit of syncResident: the head preview goes, and the work
    /// indicator unless the resident is queued again.
    fn finish_pass(&mut self, id: Uuid) -> Flow {
        let queued = self.queued(id);
        let r = self.resident_mut(id);
        r.head_preview.clear();
        if !queued {
            r.work_active = false;
            self.clear_priority(id);
        }
        self.rt.active_resident = None;
        self.publish_snapshot(id);
        self.continue_at(Phase::Top)
    }

    /// Port of handleRequestFailure (CS:1148-1167): 429 has no retry limit
    /// but an account-wide cooldown; other retryable failures one retry.
    fn handle_request_failure(&mut self, id: Uuid, status: u16, now: Instant) {
        let priority = self.rt.priority_resident == Some(id);
        if status == 429 {
            self.hold_network_until(now + RATE_LIMIT_DELAY);
            self.queue_resident(id, priority);
            return;
        }
        let r = self.resident_mut(id);
        if retryable(status) && !r.retry_used {
            r.retry_used = true;
            self.hold_network_until(now + RETRY_DELAY);
            self.queue_resident(id, priority);
        }
    }
}
