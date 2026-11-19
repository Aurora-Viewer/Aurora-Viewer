//! Opt-in, short-lived diagnostics for the whole object movement path.
//! A changed request-file timestamp starts twenty seconds of tracing. No
//! object names, UUIDs, absolute positions or network payloads are recorded.
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

static ACTIVE: AtomicBool = AtomicBool::new(false);
static TRACE: OnceLock<Option<Mutex<Trace>>> = OnceLock::new();

struct Trace {
    path: PathBuf,
    checked: Instant,
    request: Option<SystemTime>,
    until: Instant,
    slots: VecDeque<usize>,
}

fn state() -> Option<&'static Mutex<Trace>> {
    TRACE
        .get_or_init(|| {
            let path = std::env::var_os("AURORA_OBJECT_MOTION_TRACE")?;
            let now = Instant::now();
            log::info!("object motion trace: armed");
            Some(Mutex::new(Trace {
                path: path.into(),
                checked: now,
                request: None,
                until: now,
                slots: VecDeque::new(),
            }))
        })
        .as_ref()
}

/// Called once per frame; file metadata is checked at most four times a
/// second, only in an explicitly instrumented run.
pub fn poll() {
    let Some(state) = state() else { return };
    let Ok(mut state) = state.lock() else { return };
    let now = Instant::now();
    if now.duration_since(state.checked) >= Duration::from_millis(250) {
        state.checked = now;
        let request = std::fs::metadata(&state.path).and_then(|m| m.modified()).ok();
        if request.is_some() && request != state.request {
            state.until = now + Duration::from_secs(20);
            state.slots.clear();
            ACTIVE.store(true, Ordering::Relaxed);
            log::info!("object motion trace: begin duration=20s");
        }
        state.request = request;
    }
    if now >= state.until && ACTIVE.swap(false, Ordering::Relaxed) {
        log::info!("object motion trace: end");
        state.slots.clear();
    }
}

/// Retain the most recently reported volumes, bounded independently of
/// region size. Slots identify this local recording only.
pub fn watch(idx: usize) -> bool {
    if !ACTIVE.load(Ordering::Relaxed) {
        return false;
    }
    let Some(state) = state() else { return false };
    let Ok(mut state) = state.lock() else { return false };
    if let Some(i) = state.slots.iter().position(|&slot| slot == idx) {
        state.slots.remove(i);
    } else if state.slots.len() == 64 {
        state.slots.pop_front();
    }
    state.slots.push_back(idx);
    true
}

pub fn contains(idx: usize) -> bool {
    ACTIVE.load(Ordering::Relaxed) && state().and_then(|s| s.lock().ok()).is_some_and(|s| s.slots.contains(&idx))
}

pub fn slots() -> Vec<usize> {
    if !ACTIVE.load(Ordering::Relaxed) {
        return Vec::new();
    }
    state()
        .and_then(|s| s.lock().ok())
        .map_or_else(Vec::new, |s| s.slots.iter().copied().collect())
}
