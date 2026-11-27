use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;

const OVERLOAD_BURST: Duration = Duration::from_millis(500);
const RECOVERY_WINDOW: Duration = Duration::from_secs(2);
const SEND_SPACING: Duration = Duration::from_millis(50);
/// One full viewer background pool. Pace it before feedback is available.
const STARTUP_ATTEMPTS: u8 = 19;

struct State {
    startup_remaining: u8,
    last_overload: Option<Instant>,
    recovery_until: Option<Instant>,
    next_dispatch: Option<Instant>,
}

/// Shares overload feedback across background folder and subtree attempts, including
/// retries, on one capability. Interactive folders and outfit reads have
/// their own queue and do not wait here.
pub(crate) struct SubtreePacer {
    cap: String,
    state: Mutex<State>,
}

impl SubtreePacer {
    pub(crate) fn for_cap(slot: &mut Option<Arc<Self>>, cap: &str) -> Arc<Self> {
        if let Some(current) = slot.as_ref().filter(|current| current.cap == cap) {
            return current.clone();
        }
        let pacer = Arc::new(Self {
            cap: cap.to_owned(),
            state: Mutex::new(State {
                startup_remaining: STARTUP_ATTEMPTS,
                last_overload: None,
                recovery_until: None,
                next_dispatch: None,
            }),
        });
        *slot = Some(pacer.clone());
        pacer
    }

    pub(super) async fn wait(&self) {
        while let Some(next) = self.admit(Instant::now()) {
            tokio::time::sleep_until(next).await;
        }
    }

    fn admit(&self, now: Instant) -> Option<Instant> {
        let mut state = self.state.lock();
        if state.startup_remaining == 0 && state.recovery_until.is_none_or(|until| now >= until) {
            return None;
        }
        if let Some(next) = state.next_dispatch.filter(|next| now < *next) {
            return Some(next);
        }
        state.startup_remaining = state.startup_remaining.saturating_sub(1);
        state.next_dispatch = Some(now + SEND_SPACING);
        None
    }

    pub(super) fn overloaded(&self, retry_after: Option<Duration>) {
        self.overloaded_at(Instant::now(), retry_after);
    }

    fn overloaded_at(&self, now: Instant, retry_after: Option<Duration>) {
        let mut state = self.state.lock();
        let burst = state
            .last_overload
            .replace(now)
            .is_some_and(|previous| now.saturating_duration_since(previous) <= OVERLOAD_BURST);
        let delay = retry_after.unwrap_or_default().min(Duration::from_secs(60));
        let next = now + delay;
        if state.recovery_until.is_some_and(|until| until > now) || burst || !delay.is_zero() {
            state.next_dispatch = Some(state.next_dispatch.map_or(next, |pending| pending.max(next)));
            let until = next + RECOVERY_WINDOW;
            state.recovery_until = Some(state.recovery_until.map_or(until, |pending| pending.max(until)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finish_startup(pace: &SubtreePacer, start: Instant) -> Instant {
        for index in 0..u32::from(STARTUP_ATTEMPTS) {
            assert_eq!(pace.admit(start + SEND_SPACING * index), None);
        }
        start + SEND_SPACING * u32::from(STARTUP_ATTEMPTS)
    }

    #[test]
    fn first_wave_is_spread_without_pacing_healthy_later_attempts() {
        let pace = SubtreePacer::for_cap(&mut None, "new");
        let start = Instant::now();
        assert_eq!(pace.admit(start), None);
        assert_eq!(pace.admit(start), Some(start + SEND_SPACING));
        for index in 1..u32::from(STARTUP_ATTEMPTS) {
            assert_eq!(pace.admit(start + SEND_SPACING * index), None);
        }
        let last = start + SEND_SPACING * u32::from(STARTUP_ATTEMPTS - 1);
        assert_eq!(pace.admit(last), None);
        assert_eq!(pace.admit(last), None);
    }

    #[test]
    fn subtree_pacing_spreads_a_rejection_burst_then_recovers() {
        let pace = SubtreePacer::for_cap(&mut None, "a");
        let start = finish_startup(&pace, Instant::now());
        assert_eq!(pace.admit(start), None);
        assert_eq!(pace.admit(start), None);
        pace.overloaded_at(start, None);
        assert_eq!(pace.admit(start), None, "an isolated rejection only backs off its own request");
        let burst = start + Duration::from_millis(40);
        pace.overloaded_at(burst, None);
        assert_eq!(pace.admit(burst), None);
        assert_eq!(pace.admit(burst), Some(burst + SEND_SPACING));
        assert_eq!(pace.admit(burst + SEND_SPACING), None);
        assert_eq!(pace.admit(burst + RECOVERY_WINDOW), None);
        assert_eq!(pace.admit(burst + RECOVERY_WINDOW), None);
    }

    #[test]
    fn subtree_pacing_respects_retry_after_and_is_scoped_to_the_current_capability() {
        let mut slot = None;
        let old = SubtreePacer::for_cap(&mut slot, "old");
        let now = Instant::now();
        old.overloaded_at(now, Some(Duration::from_secs(5)));
        assert_eq!(old.admit(now), Some(now + Duration::from_secs(5)));
        old.overloaded_at(now + Duration::from_millis(10), None);
        assert_eq!(old.admit(now + Duration::from_secs(1)), Some(now + Duration::from_secs(5)));
        assert!(Arc::ptr_eq(&old, &SubtreePacer::for_cap(&mut slot, "old")));
        let changed = SubtreePacer::for_cap(&mut slot, "new");
        assert_eq!(changed.admit(now), None);
        assert_eq!(old.admit(now + Duration::from_secs(5)), None);
        assert_eq!(
            old.admit(now + Duration::from_secs(5)),
            Some(now + Duration::from_secs(5) + SEND_SPACING)
        );
    }
}
