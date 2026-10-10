//! Helper threads of the render thread.
//!
//! `CommandEncoder::finish` replays, validates and tracks every command of
//! its encoder, and is the largest CPU cost of a frame; the frame's three
//! encoders are finished at once. Two threads spawned for that every frame
//! cost a thread creation each and started cold, on whatever core the
//! system found: these two stay, parked on their channel between frames.
//!
//! A helper that dies (its work panicked) is not replaced: its jobs are
//! then done by the caller, and the job it died on is reported missing.

use std::sync::mpsc;
use std::thread::JoinHandle;

struct Helper<J, R> {
    /// None once dropped (the helper's loop ends when it is).
    jobs: Option<mpsc::Sender<J>>,
    done: mpsc::Receiver<R>,
    thread: Option<JoinHandle<()>>,
}

/// A job handed to helper `i`, or already done here.
enum Pending<R> {
    Sent(usize),
    Done(R),
}

pub(crate) struct Helpers<J: Send + 'static, R: Send + 'static> {
    workers: Vec<Helper<J, R>>,
    work: fn(J) -> R,
}

impl<J: Send + 'static, R: Send + 'static> Helpers<J, R> {
    /// `count` threads named `name-<i>`, each running `work` on the jobs it
    /// is handed. A thread that cannot be started is simply missing.
    pub fn new(count: usize, name: &str, work: fn(J) -> R) -> Self {
        let workers = (0..count)
            .filter_map(|i| {
                let (jobs, inbox) = mpsc::channel::<J>();
                let (outbox, done) = mpsc::channel::<R>();
                let thread = std::thread::Builder::new()
                    .name(format!("{name}-{i}"))
                    .spawn(move || {
                        for job in inbox {
                            if outbox.send(work(job)).is_err() {
                                break;
                            }
                        }
                    })
                    .map_err(|e| log::warn!("{name}-{i} not started: {e}"))
                    .ok()?;
                Some(Helper {
                    jobs: Some(jobs),
                    done,
                    thread: Some(thread),
                })
            })
            .collect();
        Helpers { workers, work }
    }

    fn hand(&self, i: usize, job: J) -> Pending<R> {
        let Some(jobs) = self.workers.get(i).and_then(|w| w.jobs.as_ref()) else {
            return Pending::Done((self.work)(job));
        };
        match jobs.send(job) {
            Ok(()) => Pending::Sent(i),
            // the helper is gone: its job is done here
            Err(mpsc::SendError(job)) => Pending::Done((self.work)(job)),
        }
    }

    fn take(&self, p: Pending<R>) -> Option<R> {
        match p {
            Pending::Done(r) => Some(r),
            Pending::Sent(i) => self.workers.get(i)?.done.recv().ok(),
        }
    }

    /// Run `a` and `b` on the helpers while `own` runs here; the results in
    /// that order. None for a job whose helper died on it.
    pub fn pair<O>(&self, a: J, b: J, own: impl FnOnce() -> O) -> (Option<R>, Option<R>, O) {
        let (a, b) = (self.hand(0, a), self.hand(1, b));
        let own = own();
        (self.take(a), self.take(b), own)
    }
}

impl<J: Send + 'static, R: Send + 'static> Drop for Helpers<J, R> {
    fn drop(&mut self) {
        for w in &mut self.workers {
            // the helper's loop ends with its channel
            w.jobs = None;
        }
        for w in &mut self.workers {
            if let Some(t) = w.thread.take() {
                let _ = t.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(n: u64) -> u64 {
        if n == 13 {
            // a job that brings its helper down
            std::panic::resume_unwind(Box::new("unlucky"));
        }
        n * n
    }

    #[test]
    fn the_three_jobs_run_and_come_back_in_order() {
        let h = Helpers::new(2, "test-helper", square);
        for n in 20..70u64 {
            assert_eq!(h.pair(n, n + 1, || n + 2), (Some(n * n), Some((n + 1) * (n + 1)), n + 2));
        }
    }

    #[test]
    fn the_helpers_work_while_the_caller_does() {
        fn name(_: ()) -> String {
            std::thread::current().name().unwrap_or_default().to_owned()
        }
        let h = Helpers::new(2, "test-helper", name);
        let here = std::thread::current().id();
        let (a, b, own) = h.pair((), (), || std::thread::current().id());
        assert_eq!(a.as_deref(), Some("test-helper-0"));
        assert_eq!(b.as_deref(), Some("test-helper-1"));
        assert_eq!(own, here);
    }

    #[test]
    fn a_dead_helper_is_reported_then_replaced_by_the_caller() {
        let h = Helpers::new(2, "test-helper", square);
        // the job it dies on is missing, the others still come back
        assert_eq!(h.pair(13, 3, || 1), (None, Some(9), 1));
        // from then on its jobs are done by the caller: no frame is lost
        assert_eq!(h.pair(4, 5, || 2), (Some(16), Some(25), 2));
        assert_eq!(h.pair(6, 7, || 3), (Some(36), Some(49), 3));
    }

    #[test]
    fn without_helpers_the_caller_does_everything() {
        let h = Helpers::new(0, "test-helper", square);
        assert_eq!(h.pair(2, 3, || 4), (Some(4), Some(9), 4));
    }
}
