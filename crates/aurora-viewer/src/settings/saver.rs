//! The settings file written off the frame thread. Each write now flushes
//! the file to the disk (atomic_file.rs, #1809), which can take a few
//! milliseconds and more on a busy disk; the settings are saved from the
//! frame thread at every change of an option. `Settings::save` takes the
//! copy at once and hands its write here: one thread writes the saves in
//! their order, and a save not started yet is replaced by a newer one (it
//! held an older state of the same file). The viewer waits for the last one
//! before it ends (`wait`).
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

pub(super) type Job = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct State {
    pending: Option<Job>,
    writing: bool,
}

#[derive(Default)]
pub(super) struct Saver {
    state: Mutex<State>,
    changed: Condvar,
    /// Whether the save thread runs; without it the saves are written by
    /// the caller, as before.
    started: OnceLock<bool>,
}

impl Saver {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn queue(self: &Arc<Self>, job: Job) {
        let running = *self.started.get_or_init(|| {
            let saver = Arc::clone(self);
            std::thread::Builder::new()
                .name("aurora-settings-save".into())
                .spawn(move || saver.run())
                .map_err(|e| log::warn!("settings saved on the frame thread: no save thread ({e})"))
                .is_ok()
        });
        if !running {
            job();
            return;
        }
        self.lock().pending = Some(job);
        self.changed.notify_all();
    }

    fn run(&self) {
        loop {
            let job = {
                let mut state = self.lock();
                loop {
                    if let Some(job) = state.pending.take() {
                        state.writing = true;
                        break job;
                    }
                    state = self.changed.wait(state).unwrap_or_else(|poisoned| poisoned.into_inner());
                }
            };
            // (a panic is logged by the hook; the next saves still go)
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
            self.lock().writing = false;
            self.changed.notify_all();
        }
    }

    /// Until every save queued is written, `timeout` at most (a disk that
    /// does not answer): false when some were left.
    pub(super) fn wait(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.lock();
        while state.pending.is_some() || state.writing {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            state = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        true
    }
}

/// The settings file's saver.
pub(super) fn get() -> &'static Arc<Saver> {
    static SAVER: OnceLock<Arc<Saver>> = OnceLock::new();
    SAVER.get_or_init(Default::default)
}

#[cfg(test)]
mod tests {
    use super::Saver;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Barrier, Mutex};
    use std::time::Duration;

    #[test]
    fn a_save_not_started_is_replaced_by_the_newer_one() {
        let saver = Arc::new(Saver::default());
        let written = Arc::new(Mutex::new(Vec::new()));
        let started = Arc::new(Barrier::new(2));
        let gate = Arc::new(Barrier::new(2));
        // the first save holds the thread while three more are queued
        let (w, s, g) = (Arc::clone(&written), Arc::clone(&started), Arc::clone(&gate));
        saver.queue(Box::new(move || {
            s.wait();
            g.wait();
            w.lock().expect("written").push(0);
        }));
        started.wait();
        for n in 1..=3 {
            let w = Arc::clone(&written);
            saver.queue(Box::new(move || w.lock().expect("written").push(n)));
        }
        gate.wait();
        assert!(saver.wait(Duration::from_secs(10)));
        assert_eq!(*written.lock().expect("written"), vec![0, 3]);
    }

    #[test]
    fn wait_returns_once_the_last_save_is_written() {
        let saver = Arc::new(Saver::default());
        assert!(saver.wait(Duration::ZERO), "nothing queued: nothing to wait for");
        let done = Arc::new(AtomicU32::new(0));
        let d = Arc::clone(&done);
        saver.queue(Box::new(move || {
            std::thread::sleep(Duration::from_millis(50));
            d.store(1, Ordering::SeqCst);
        }));
        assert!(saver.wait(Duration::from_secs(10)));
        assert_eq!(done.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn wait_gives_up_after_its_timeout() {
        let saver = Arc::new(Saver::default());
        let gate = Arc::new(Barrier::new(2));
        let g = Arc::clone(&gate);
        saver.queue(Box::new(move || {
            g.wait();
        }));
        assert!(!saver.wait(Duration::from_millis(20)));
        gate.wait();
        assert!(saver.wait(Duration::from_secs(10)));
    }

    #[test]
    fn a_save_that_panics_does_not_stop_the_next() {
        let saver = Arc::new(Saver::default());
        saver.queue(Box::new(|| panic!("a save that fails")));
        assert!(saver.wait(Duration::from_secs(10)));
        let done = Arc::new(AtomicU32::new(0));
        let d = Arc::clone(&done);
        saver.queue(Box::new(move || d.store(1, Ordering::SeqCst)));
        assert!(saver.wait(Duration::from_secs(10)));
        assert_eq!(done.load(Ordering::SeqCst), 1);
    }
}
