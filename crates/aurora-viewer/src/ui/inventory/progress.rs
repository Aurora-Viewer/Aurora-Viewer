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

#[derive(Clone)]
pub(super) struct Notice {
    request: Uuid,
    text: String,
    /// None while waiting; success/error share the usual five-second lifetime.
    result: Option<(bool, Instant)>,
}

impl InventoryUi {
    pub fn begin_contents_copy(&mut self, request: Uuid, text: String) {
        self.message.clear();
        if self.thumbnail_notice.as_ref().is_some_and(|notice| notice.result.is_some()) {
            self.thumbnail_notice = None;
        }
        self.contents_notice = Some(Notice {
            request,
            text,
            result: None,
        });
    }

    pub fn finish_contents_copy(&mut self, request: Uuid, text: String, success: bool, now: Instant) {
        if let Some(notice) = &mut self.contents_notice
            && notice.request == request
        {
            notice.text = text;
            notice.result = Some((success, now));
        }
    }

    pub(super) fn contents_notice(&self, now: Instant) -> Option<(&str, Option<bool>)> {
        self.contents_notice
            .as_ref()
            .filter(|notice| {
                notice
                    .result
                    .is_none_or(|(_, finished)| now.saturating_duration_since(finished) < NOTICE_DURATION)
            })
            .map(|notice| (notice.text.as_str(), notice.result.map(|(success, _)| success)))
    }

    /// Both inventory presentations use the same priority, colors and expiry.
    pub(super) fn status_message(&self, p: &crate::theme::Palette, now: Instant) -> Option<(String, egui::Color32)> {
        if !self.message.is_empty() {
            Some((self.message.clone(), p.warn))
        } else if let Some(notice) = self.thumbnail_notice.as_ref().filter(|notice| {
            notice
                .result
                .is_none_or(|(_, finished)| now.saturating_duration_since(finished) < NOTICE_DURATION)
        }) {
            Some((
                notice.text.clone(),
                match notice.result {
                    None => p.ink,
                    Some((true, _)) => p.success,
                    Some((false, _)) => p.warn,
                },
            ))
        } else if self.busy() {
            Some((self.pending_status(now), p.ink))
        } else if let Some((text, result)) = self.contents_notice(now) {
            Some((
                text.to_owned(),
                match result {
                    None => p.ink,
                    Some(true) => p.success,
                    Some(false) => p.warn,
                },
            ))
        } else if let Some(text) = self.success_notice(now) {
            Some((text.to_owned(), p.success))
        } else if self.fetch_all {
            Some((aurora_i18n::tr!("ui-inventory-search-loading-folders").to_owned(), p.muted))
        } else {
            None
        }
    }

    pub(crate) fn begin_thumbnail_upload(&mut self, request: Uuid) {
        self.message.clear();
        self.notice = None;
        self.thumbnail_notice = Some(Notice {
            request,
            text: aurora_i18n::tr!("ui-inventory-thumbnail-processing-image").into(),
            result: None,
        });
        for window in &mut self.windows {
            window.state.begin_thumbnail_upload(request);
        }
    }

    pub(crate) fn finish_thumbnail_upload(&mut self, request: Uuid, error: Option<&str>, now: Instant) {
        if self.thumbnail_notice.as_ref().is_some_and(|notice| notice.request != request) {
            return;
        }
        self.notice = None;
        self.thumbnail_notice = Some(Notice {
            request,
            text: error.map_or_else(|| aurora_i18n::tr!("inventory-thumbnail-upload-complete").into(), str::to_owned),
            result: Some((error.is_none(), now)),
        });
        for window in &mut self.windows {
            window.state.finish_thumbnail_upload(request, error, now);
        }
    }

    pub fn begin_mutation(&mut self, request: Uuid, change: &Mutation) {
        self.thumbnail_notice = None;
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
    fn thumbnail_loading_waits_for_a_result_then_its_footer_expires() {
        for success in [false, true] {
            let now = Instant::now();
            let request = Uuid::from_u128(1);
            let p = crate::theme::Theme::default().palette();
            let mut st = InventoryUi {
                message: "Old warning".into(),
                ..Default::default()
            };
            st.windows.push(super::super::InventoryWindow {
                id: Uuid::from_u128(10),
                root: Uuid::nil(),
                additional: true,
                state: Box::default(),
                open: true,
            });
            st.pending = Some(request);
            st.begin_thumbnail_upload(request);
            let loading = aurora_i18n::tr!("ui-inventory-thumbnail-processing-image").to_owned();
            assert_eq!(st.status_message(&p, now + Duration::from_secs(60)), Some((loading.clone(), p.ink)));
            assert_eq!(st.windows[0].state.status_message(&p, now), Some((loading, p.ink)));
            st.finish_mutation(success);
            let error = (!success).then_some("Image could not be verified");
            st.finish_thumbnail_upload(request, error, now);
            let complete = error.map_or_else(|| aurora_i18n::tr!("inventory-thumbnail-upload-complete").to_owned(), str::to_owned);
            for state in [&st, st.windows[0].state.as_ref()] {
                assert_eq!(
                    state.status_message(&p, now + Duration::from_millis(4999)),
                    Some((complete.clone(), if success { p.success } else { p.warn }))
                );
                assert!(state.status_message(&p, now + NOTICE_DURATION).is_none());
                assert!(state.message.is_empty(), "no permanent warning remains");
            }
        }
    }

    #[test]
    fn an_older_thumbnail_result_cannot_replace_the_current_loading_state() {
        let now = Instant::now();
        let p = crate::theme::Theme::default().palette();
        let mut st = InventoryUi::default();
        st.begin_thumbnail_upload(Uuid::from_u128(1));
        st.begin_thumbnail_upload(Uuid::from_u128(2));
        st.finish_thumbnail_upload(Uuid::from_u128(1), Some("Old failure"), now);
        assert_eq!(
            st.status_message(&p, now),
            Some((aurora_i18n::tr!("ui-inventory-thumbnail-processing-image").into(), p.ink))
        );
        st.finish_thumbnail_upload(Uuid::from_u128(2), None, now);
        assert_eq!(
            st.status_message(&p, now),
            Some((aurora_i18n::tr!("inventory-thumbnail-upload-complete").into(), p.success))
        );
        st.finish_thumbnail_upload(Uuid::from_u128(1), Some("Late failure"), now + Duration::from_secs(6));
        assert!(st.status_message(&p, now + Duration::from_secs(6)).is_none());
    }

    #[test]
    fn a_new_operation_replaces_the_previous_thumbnail_result() {
        let now = Instant::now();
        let p = crate::theme::Theme::default().palette();
        for contents in [false, true] {
            let mut st = InventoryUi::default();
            st.begin_thumbnail_upload(Uuid::from_u128(1));
            st.finish_thumbnail_upload(Uuid::from_u128(1), None, now);
            if contents {
                st.begin_contents_copy(Uuid::from_u128(2), "Requested".into());
                assert_eq!(st.status_message(&p, now), Some(("Requested".into(), p.ink)));
            } else {
                st.begin_mutation(Uuid::from_u128(2), &Mutation::default());
                assert_eq!(st.status_message(&p, now), Some((st.pending_status(now), p.ink)));
            }
        }
    }

    #[test]
    fn contents_copy_pending_is_replaced_by_success_or_error_then_expires() {
        for success in [false, true] {
            let now = Instant::now();
            let request = Uuid::from_u128(1);
            let mut st = InventoryUi::default();
            let p = crate::theme::Theme::default().palette();
            st.begin_contents_copy(request, "Requested".into());
            assert_eq!(st.contents_notice(now + Duration::from_secs(60)), Some(("Requested", None)));
            assert_eq!(st.status_message(&p, now), Some(("Requested".into(), p.ink)));
            st.finish_contents_copy(request, "Result".into(), success, now);
            assert_eq!(
                st.status_message(&p, now),
                Some(("Result".into(), if success { p.success } else { p.warn }))
            );
            assert_eq!(
                st.contents_notice(now + Duration::from_millis(4999)),
                Some(("Result", Some(success)))
            );
            assert!(st.contents_notice(now + NOTICE_DURATION).is_none());
            assert!(st.status_message(&p, now + NOTICE_DURATION).is_none());
            assert!(st.message.is_empty(), "no permanent warning remains after expiry");
        }
    }

    #[test]
    fn an_older_contents_copy_cannot_overwrite_a_newer_request_or_an_unrelated_error() {
        let now = Instant::now();
        let mut st = InventoryUi::default();
        st.begin_contents_copy(Uuid::from_u128(1), "First".into());
        st.begin_contents_copy(Uuid::from_u128(2), "Second".into());
        st.finish_contents_copy(Uuid::from_u128(1), "First failed".into(), false, now);
        assert_eq!(st.contents_notice(now), Some(("Second", None)));
        st.message = "Unrelated error".into();
        st.finish_contents_copy(Uuid::from_u128(2), "Second succeeded".into(), true, now);
        assert_eq!(st.message, "Unrelated error");
        assert_eq!(st.contents_notice(now), Some(("Second succeeded", Some(true))));
    }

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
