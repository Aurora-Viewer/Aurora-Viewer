//! "Delete transcripts": the durable cutoff first, then the views, then
//! the files. Port of deleteTranscriptsAsync, runDelete and finishDelete
//! (indra/newview/llchatservicehistory.cpp, LL viewer 26.5).

use super::Action;
use super::Manager;
use super::manager::{Flow, MetaState, Phase, StateSafety, Step};

impl Manager {
    /// The user confirmed the delete; `click_ticks` comes from
    /// [`super::click_ticks`]. False when not running or already deleting.
    /// History is hidden from now on, until the delete succeeds. Port of
    /// LLChatServiceHistory::deleteTranscriptsAsync.
    pub fn request_delete(&mut self, click_ticks: u64) -> bool {
        if !self.rt.running || self.rt.delete_active {
            return false;
        }
        // No new request or view read may start, and no expired quiet
        // deadline remains a wake source, before the cutoff is durable.
        self.rt.delete_click_ticks = click_ticks;
        self.rt.delete_active = true;
        self.rt.delete_requested = true;
        for r in self.rt.residents.values_mut() {
            r.activity_refresh_due = None;
        }
        self.wake();
        true
    }

    /// runDelete, step 1: the pending cutoff, before any view or file.
    pub(super) fn run_delete(&mut self) -> Flow {
        let ticket = self.ticket();
        self.rt.outbox.push(Action::BeginDeleteState {
            ticket,
            click_ticks: self.rt.delete_click_ticks,
        });
        self.rt.step = Step::DeletePending(ticket);
        Flow::Wait
    }

    /// runDelete, steps 2 and 3: cancel the synchronization, clear every
    /// historical owner, then sweep.
    pub(super) fn delete_pending_written(&mut self, boundary: Option<u64>) -> Flow {
        let Some(boundary) = boundary else {
            self.rt.state_safety = StateSafety::Unsafe;
            return self.finish_delete(false);
        };
        self.rt.deleted_before_ticks = boundary;
        self.rt.cleanup_pending = true;
        self.rt.queue.clear();
        self.rt.active_resident = None;
        self.rt.priority_resident = None;
        let ids: Vec<_> = self.rt.residents.keys().copied().collect();
        for id in ids {
            let r = self.resident_mut(id);
            // Disconnected: a late name answer is dropped.
            if let MetaState::Pending { subscribed, .. } = &mut r.metadata {
                *subscribed = false;
            }
            r.head_preview.clear();
            r.work_active = false;
            self.publish_snapshot(id);
        }
        // The main-loop clear (with its style update) runs before the
        // sweep: a late clear must not drop messages received after it.
        self.rt.outbox.push(Action::ClearSessionsForDeletion);
        self.rt.outbox.push(Action::StyleUpdate);
        let ticket = self.ticket();
        self.rt.outbox.push(Action::SweepTranscripts { ticket });
        self.rt.step = Step::DeleteSweep { ticket, boundary };
        Flow::Wait
    }

    /// runDelete, step 5: pending is cleared only after both sweeps.
    pub(super) fn delete_swept(&mut self, boundary: u64, ok: bool) -> Flow {
        if !ok {
            return self.finish_delete(false);
        }
        let ticket = self.ticket();
        self.rt.outbox.push(Action::WriteState {
            ticket,
            boundary,
            cleanup_pending: false,
        });
        self.rt.step = Step::DeleteFinal { ticket };
        Flow::Wait
    }

    /// runDelete, step 6: the summaries go last, so that the discovery
    /// starts again from empty storage.
    pub(super) fn delete_final_written(&mut self, ok: bool) -> Flow {
        if !ok {
            self.rt.state_safety = StateSafety::Unsafe;
            return self.finish_delete(false);
        }
        self.rt.residents.clear();
        self.rt.initialized_archives = true;
        self.rt.local_content_exists = false;
        self.finish_delete(true)
    }

    /// Port of finishDelete (CS:1639-1669): a failure keeps history hidden
    /// for this login.
    fn finish_delete(&mut self, success: bool) -> Flow {
        self.rt.delete_active = false;
        if success {
            self.rt.delete_requested = false;
            self.rt.cleanup_pending = false;
            self.rt.state_safety = StateSafety::Safe;
            self.rt.list_needed = true;
        } else {
            self.rt.delete_requested = true;
        }
        self.rt.outbox.push(Action::DeleteFinished { success });
        self.rt.outbox.push(Action::TranscriptCreated);
        self.continue_at(Phase::Top)
    }
}
