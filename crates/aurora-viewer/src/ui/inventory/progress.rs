//! Inventory mutation feedback: actual server counts and elapsed time.
use super::InventoryUi;
use aurora_net::inventory::operations::{Mutation, Phase, Progress};
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Clone)]
pub(super) struct State {
    value: Progress,
    started: Instant,
}

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
            return;
        }
        if let Some(progress) = &mut self.progress {
            progress.value = value;
        }
    }

    pub fn finish_mutation(&mut self, success: bool) {
        self.pending = None;
        self.progress = None;
        self.notice = success.then(|| (aurora_i18n::tr!("inventory-operation-complete").into(), Instant::now()));
    }

    pub(super) fn pending_status(&self, now: Instant) -> String {
        let Some(progress) = &self.progress else {
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

pub(super) const NOTICE_DURATION: Duration = Duration::from_secs(8);

#[cfg(test)]
mod tests {
    use super::*;

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
