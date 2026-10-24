//! Inventory mutation feedback: actual server counts and elapsed time.
use super::InventoryUi;
use aurora_net::inventory::operations::{Mutation, Phase, Progress, Reply};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Clone)]
pub(super) struct State {
    value: Progress,
    started: Instant,
}

#[derive(Clone)]
pub(super) struct CopyJob {
    progress: State,
    refresh: Vec<Uuid>,
}

pub(super) type CopyJobs = HashMap<Uuid, CopyJob>;

impl InventoryUi {
    pub fn begin_mutation(&mut self, request: Uuid, change: &Mutation) {
        self.pending = Some(request);
        self.progress = Some(State {
            value: if change.operations.is_empty() {
                Progress {
                    phase: Phase::Copying,
                    completed: 0,
                    total: change.copies.len(),
                }
            } else {
                Progress {
                    phase: Phase::Updating,
                    completed: 0,
                    total: change.operations.len(),
                }
            },
            started: Instant::now(),
        });
        self.notice = None;
    }

    pub fn update_mutation_progress(&mut self, request: Uuid, value: Progress) {
        if self.pending != Some(request) {
            if let Some(job) = self.copy_jobs.get_mut(&request) {
                job.progress.value = value;
            }
            return;
        }
        if let Some(progress) = &mut self.progress {
            progress.value = value;
        }
    }

    pub fn watch_copy(&mut self, request: Uuid, reply: &Reply) {
        let mut progress = self.progress.take().unwrap_or(State {
            value: Progress {
                phase: Phase::Copying,
                completed: 0,
                total: reply.copies.len(),
            },
            started: Instant::now(),
        });
        progress.value = Progress {
            phase: Phase::Copying,
            completed: 0,
            total: reply.copies.len(),
        };
        self.copy_jobs.insert(
            request,
            CopyJob {
                progress,
                refresh: reply.refresh.clone(),
            },
        );
        // Only submission is serialized. Copies arrive in the background, as
        // in Firestorm, and must not disable unrelated inventory commands.
        self.pending = None;
        self.pending_refresh.clear();
        self.created_selection = None;
        self.consume_clipboard = false;
    }

    pub fn finish_copy(&mut self, request: Uuid, success: bool) -> Option<Vec<Uuid>> {
        let job = self.copy_jobs.remove(&request)?;
        if self.copy_jobs.is_empty() && self.pending.is_none() {
            self.notice = success.then(|| (aurora_i18n::tr!("inventory-operation-complete").into(), Instant::now()));
        }
        Some(job.refresh)
    }

    pub(super) fn busy(&self) -> bool {
        self.pending.is_some() || !self.copy_jobs.is_empty()
    }

    pub fn finish_mutation(&mut self, success: bool) {
        self.pending = None;
        self.progress = None;
        self.notice = success.then(|| (aurora_i18n::tr!("inventory-operation-complete").into(), Instant::now()));
    }

    pub fn moves_queued_notice(&mut self) {
        self.notice = Some((aurora_i18n::tr!("inventory-moves-queued").into(), Instant::now()));
    }

    pub(super) fn success_notice(&self, now: Instant) -> Option<&str> {
        self.notice
            .as_ref()
            .filter(|(_, finished)| now.saturating_duration_since(*finished) < NOTICE_DURATION)
            .map(|(message, _)| message.as_str())
    }

    pub(super) fn pending_status(&self, now: Instant) -> String {
        let aggregate = (!self.copy_jobs.is_empty()).then(|| State {
            value: Progress {
                phase: Phase::Copying,
                completed: self.copy_jobs.values().map(|j| j.progress.value.completed).sum(),
                total: self.copy_jobs.values().map(|j| j.progress.value.total).sum(),
            },
            started: self.copy_jobs.values().map(|j| j.progress.started).min().unwrap_or(now),
        });
        let Some(progress) = self.progress.as_ref().or(aggregate.as_ref()) else {
            return aurora_i18n::tr!("ui-inventory-changing").into();
        };
        let value = progress.value;
        let message = match value.phase {
            Phase::Updating => aurora_i18n::tr!("inventory-progress-updating", completed = value.completed, total = value.total),
            Phase::Copying => aurora_i18n::tr!("inventory-progress-copying", completed = value.completed, total = value.total),
            Phase::Verifying => aurora_i18n::tr!("inventory-progress-verifying").to_owned(),
        };
        aurora_i18n::tr!(
            "inventory-progress-elapsed",
            message = message,
            seconds = now.duration_since(progress.started).as_secs()
        )
    }
}

pub(super) fn demo(frame: u64, st: &mut InventoryUi) {
    let request = Uuid::from_u128(0xc11b);
    if frame == 650 {
        st.begin_mutation(request, &Mutation::default());
    }
    let phase = match frame {
        650 => Some((Phase::Updating, 0)),
        700 => Some((Phase::Updating, 2)),
        750 => Some((Phase::Copying, 0)),
        800 => Some((Phase::Copying, 2)),
        850 => Some((Phase::Verifying, 3)),
        _ => None,
    };
    if let Some((phase, completed)) = phase {
        st.update_mutation_progress(
            request,
            Progress {
                phase,
                completed,
                total: 3,
            },
        );
    }
    if frame == 920 {
        st.finish_mutation(true);
    }
}

const NOTICE_DURATION: Duration = Duration::from_secs(5);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_notices_expire_after_five_seconds_and_a_new_notice_restarts_the_duration() {
        let now = Instant::now();
        let mut st = InventoryUi {
            notice: Some(("Completed".into(), now)),
            ..Default::default()
        };
        assert_eq!(st.success_notice(now + Duration::from_millis(4999)), Some("Completed"));
        assert!(st.success_notice(now + Duration::from_secs(5)).is_none());
        st.notice = Some(("Restored".into(), now + Duration::from_secs(4)));
        assert_eq!(st.success_notice(now + Duration::from_secs(5)), Some("Restored"));
        assert!(st.success_notice(now + Duration::from_secs(9)).is_none());
    }

    #[test]
    fn outstanding_copies_leave_commands_available_and_track_multiple_jobs() {
        let mut st = InventoryUi::default();
        let reply = Reply {
            contents: Vec::new(),
            folders: Vec::new(),
            moves_queued: false,
            refresh: vec![Uuid::from_u128(10)],
            copies: vec![aurora_net::inventory::operations::CopyItem {
                item: Uuid::from_u128(3),
                owner: Uuid::from_u128(2),
                parent: Uuid::from_u128(10),
                name: String::new(),
            }],
            removed: Vec::new(),
            created: Default::default(),
        };
        for request in [Uuid::from_u128(1), Uuid::from_u128(2)] {
            assert!(st.pending.is_none(), "the previous copy does not block another command");
            st.begin_mutation(request, &Mutation::default());
            st.watch_copy(request, &reply);
            assert!(st.pending.is_none() && st.busy());
        }
        assert_eq!(st.copy_jobs.len(), 2);
        st.finish_copy(Uuid::from_u128(1), true).expect("first job");
        assert!(st.busy() && st.notice.is_none());
        st.finish_copy(Uuid::from_u128(2), true).expect("second job");
        assert!(!st.busy() && st.notice.is_some());
        st.update_mutation_progress(
            Uuid::from_u128(1),
            Progress {
                phase: Phase::Copying,
                completed: 1,
                total: 1,
            },
        );
        assert!(!st.busy());
    }

    #[test]
    fn progress_is_immediate_and_late_confirmations_cannot_reopen_a_completed_operation() {
        let request = Uuid::from_u128(1);
        let mut st = InventoryUi::default();
        st.begin_mutation(request, &Mutation::default());
        assert!(st.progress.is_some());
        let value = Progress {
            phase: Phase::Copying,
            completed: 2,
            total: 5,
        };
        st.update_mutation_progress(Uuid::from_u128(2), value);
        assert_eq!(st.progress.as_ref().expect("pending").value.completed, 0);
        st.update_mutation_progress(request, value);
        let started = st.progress.as_ref().expect("pending").started;
        let label = st.pending_status(started + Duration::from_secs(12));
        assert!(label.contains('2') && label.contains('5') && label.contains("12"));
        st.finish_mutation(true);
        assert!(st.pending.is_none() && st.progress.is_none() && st.notice.is_some());
        st.update_mutation_progress(request, value);
        assert!(st.progress.is_none());
        st.begin_mutation(Uuid::from_u128(3), &Mutation::default());
        assert!(st.notice.is_none());
        st.finish_mutation(false);
        assert!(st.notice.is_none());
    }
}
