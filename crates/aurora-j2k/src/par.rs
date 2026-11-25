//! The threads a caller lends a decode (`Options::threads`).
//!
//! The idea is neo-j2k's stage 5: the decoder owns no thread, its host
//! lends some, and only idle ones. Here the host gives a function that
//! queues a task on its own pool ([`Threads::spawn`]). For a large tile the
//! decoder queues a few tasks that offer to help, and starts on the work
//! itself: an idle thread of the pool picks a task up at once and shares
//! the work; on a busy pool the tasks wait in the queue, and by the time
//! one runs the work is done and it returns at once. So a lone texture is
//! decoded by every idle core, a queue of textures by one core each, and
//! the host has nothing to decide.
//!
//! The calling thread never waits on the pool's queue, only (on a condition
//! variable) for the tasks that did start, each of which is finishing one
//! code-block or one component. It therefore never takes another job of
//! the pool onto its stack while it waits, which a parallel loop of the
//! pool's own would do.
//!
//! Every thread takes its work from a counter the others share, so the
//! result does not depend on how many threads help, nor on when they come.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// Queues a task on the caller's pool: run on an idle thread at once, or
/// later, or never.
pub type Spawn = fn(Box<dyn FnOnce() + Send + 'static>);

/// The threads a decode may use beyond the calling one.
#[derive(Debug, Clone, Copy, Default)]
pub struct Threads {
    /// Threads a large tile's decode is shared between at most, the calling
    /// one included; 0 or 1 for the calling thread alone.
    pub count: usize,
    /// How the decoder asks for help; without it, the calling thread alone.
    pub spawn: Option<Spawn>,
}

/// A [`Spawn`] for callers without a pool (tools, tests): a thread of its
/// own for each task.
pub fn spawn_thread(task: Box<dyn FnOnce() + Send + 'static>) {
    // (a thread the system refuses is a task that never runs)
    let _ = std::thread::Builder::new().name("aurora-j2k".into()).spawn(task);
}

impl Threads {
    /// A thread of its own for each task, `count` at most with the calling
    /// one: for callers without a pool.
    pub fn os(count: usize) -> Self {
        Threads {
            count,
            spawn: Some(spawn_thread),
        }
    }

    /// Whether a tile whose largest decoded component has `samples`
    /// samples is shared between threads: from 512 x 512 on, as in
    /// neo-j2k, below which sharing costs more than it brings.
    pub(crate) fn share(&self, samples: usize) -> bool {
        self.count > 1 && self.spawn.is_some() && samples >= 1 << 18
    }
}

/// Who is at work on a shared step.
#[derive(Default)]
struct Gate {
    state: Mutex<GateState>,
    left: Condvar,
}

#[derive(Default)]
struct GateState {
    /// Tasks that started and have not returned.
    active: usize,
    /// The calling thread has finished: a task that starts now returns.
    closed: bool,
    panicked: bool,
}

/// A task's presence at the gate; it leaves when dropped, a panic included.
struct Entered(Arc<Gate>);

impl Drop for Entered {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        state.active -= 1;
        state.panicked |= std::thread::panicking();
        self.0.left.notify_all();
    }
}

/// Closes the gate when the calling thread is done with its share, a panic
/// included: a task that starts after that returns at once, and works for
/// nobody.
struct Close<'a>(&'a Gate);

impl Drop for Close<'_> {
    fn drop(&mut self) {
        self.0.state.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
    }
}

/// Runs `own(work)` on the calling thread and `step(work, i)`, `i` from 1
/// to `extra`, on the threads of the pool that are free to take them, and
/// returns when the calling thread and every task that started have
/// returned: false when one of the tasks panicked. Both take what is left
/// to do from `work`, so a task that never starts is not missed; and the
/// calling thread, which is there for sure, can work with what it borrows
/// (its own scratch, the image) where a task only has what `work` owns.
pub(crate) fn share<W: Send + Sync + 'static>(
    threads: Threads,
    extra: usize,
    work: &Arc<W>,
    step: fn(&W, usize),
    own: impl FnOnce(&W),
) -> bool {
    let gate = Arc::new(Gate::default());
    if let Some(spawn) = threads.spawn {
        for i in 1..=extra {
            let (work, gate) = (work.clone(), gate.clone());
            spawn(Box::new(move || {
                {
                    let mut state = gate.state.lock().unwrap_or_else(|e| e.into_inner());
                    if state.closed {
                        return;
                    }
                    state.active += 1;
                }
                let _entered = Entered(gate);
                step(&work, i);
            }));
        }
    }
    {
        let _close = Close(&gate);
        own(work);
    }
    let mut state = gate.state.lock().unwrap_or_else(|e| e.into_inner());
    while state.active > 0 {
        state = gate.left.wait(state).unwrap_or_else(|e| e.into_inner());
    }
    !state.panicked
}

/// The items of a step, taken one at a time by the threads that share it.
pub(crate) struct Queue {
    next: AtomicUsize,
    len: usize,
}

impl Queue {
    pub fn new(len: usize) -> Self {
        Queue {
            next: AtomicUsize::new(0),
            len,
        }
    }

    /// The next item nobody has taken, or none when all are.
    pub fn take(&self) -> Option<usize> {
        let i = self.next.fetch_add(1, Ordering::Relaxed);
        (i < self.len).then_some(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Count {
        queue: Queue,
        seen: Vec<AtomicUsize>,
    }

    fn count(work: &Count, _: usize) {
        while let Some(i) = work.queue.take() {
            work.seen[i].fetch_add(1, Ordering::Relaxed);
        }
    }

    fn counting(items: usize) -> Arc<Count> {
        Arc::new(Count {
            queue: Queue::new(items),
            seen: (0..items).map(|_| AtomicUsize::new(0)).collect(),
        })
    }

    #[test]
    fn every_item_is_taken_once_whatever_the_threads() {
        for threads in [Threads::default(), Threads::os(1), Threads::os(2), Threads::os(5), Threads::os(16)] {
            let work = counting(1000);
            assert!(share(threads, threads.count.saturating_sub(1), &work, count, |w| count(w, 0)));
            assert!(work.seen.iter().all(|n| n.load(Ordering::Relaxed) == 1), "{threads:?}");
        }
    }

    #[test]
    fn a_task_that_never_runs_is_not_waited_for() {
        fn never(_: Box<dyn FnOnce() + Send + 'static>) {}
        let work = counting(10);
        assert!(share(
            Threads {
                count: 4,
                spawn: Some(never)
            },
            3,
            &work,
            count,
            |w| count(w, 0)
        ));
        assert!(work.seen.iter().all(|n| n.load(Ordering::Relaxed) == 1));
    }

    #[test]
    fn a_task_that_comes_late_does_nothing() {
        // the tasks are kept, and run once the step is over
        static LATE: Mutex<Vec<Box<dyn FnOnce() + Send + 'static>>> = Mutex::new(Vec::new());
        fn later(task: Box<dyn FnOnce() + Send + 'static>) {
            LATE.lock().expect("no panic").push(task);
        }
        let work = counting(10);
        assert!(share(
            Threads {
                count: 3,
                spawn: Some(later)
            },
            2,
            &work,
            count,
            |w| count(w, 0)
        ));
        let late: Vec<_> = std::mem::take(&mut *LATE.lock().expect("no panic"));
        assert_eq!(late.len(), 2);
        for task in late {
            task();
        }
        assert!(work.seen.iter().all(|n| n.load(Ordering::Relaxed) == 1));
        assert_eq!(Arc::strong_count(&work), 1);
    }

    #[test]
    fn a_panic_in_a_task_is_reported() {
        // the task runs where it is queued, and its panic stops there, as
        // on a pool's thread
        fn inline(task: Box<dyn FnOnce() + Send + 'static>) {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
        }
        fn fail(_: &(), _: usize) {
            panic!("the task that fails");
        }
        let threads = Threads {
            count: 2,
            spawn: Some(inline),
        };
        assert!(!share(threads, 1, &Arc::new(()), fail, |_| {}));
    }

    #[test]
    fn a_panic_of_the_calling_thread_turns_late_tasks_away() {
        static LATE: Mutex<Vec<Box<dyn FnOnce() + Send + 'static>>> = Mutex::new(Vec::new());
        fn later(task: Box<dyn FnOnce() + Send + 'static>) {
            LATE.lock().expect("no panic").push(task);
        }
        let work = counting(10);
        let threads = Threads {
            count: 3,
            spawn: Some(later),
        };
        let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            share(threads, 2, &work, count, |_| panic!("the calling thread fails"))
        }));
        assert!(stopped.is_err());
        let late: Vec<_> = std::mem::take(&mut *LATE.lock().expect("no panic"));
        assert_eq!(late.len(), 2);
        for task in late {
            task();
        }
        assert!(
            work.seen.iter().all(|n| n.load(Ordering::Relaxed) == 0),
            "nothing is done for a caller that is gone"
        );
    }

    #[test]
    fn small_tiles_stay_on_one_thread() {
        assert!(!Threads::os(8).share(256 * 256));
        assert!(Threads::os(8).share(512 * 512));
        assert!(!Threads::default().share(4096 * 4096));
        assert!(!Threads { count: 8, spawn: None }.share(4096 * 4096));
    }
}
