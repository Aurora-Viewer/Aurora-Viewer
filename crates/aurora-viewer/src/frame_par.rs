//! Parallel loops of the frame that the calling thread takes part in.
//!
//! The main thread is not a worker of the rayon pool: a `par_iter` called
//! from it hands the whole loop to the pool and sleeps until the last
//! worker is done. A frame then waits for a worker to wake up, and to be
//! woken up itself afterwards, however small the loop; when the cores are
//! busy (decodes on arrival in a region, another program) each wake-up can
//! take milliseconds, for a loop of a few hundred microseconds (scene sync
//! planning of 2 400 objects at BackBone Main Store: 0.9 ms on average,
//! 1.3 to 9 ms in the worst frame of each second).
//!
//! Here the caller starts on the loop at once and a few workers of the pool
//! join as they wake up: chunks are claimed one at a time by whoever is
//! free. Workers that come late find nothing left and go back to sleep;
//! nobody waits for them. In the worst case the caller does the whole loop
//! alone, which costs what the loop costs. A short loop asks for no helper
//! at all: waking one costs more than it brings.
//!
//! What remains: a helper that loses its core in the middle of a chunk is
//! waited for (the loop borrows from the caller). Fewer helpers are asked
//! than the pool has workers, which also keeps that rare.

use parking_lot::Mutex;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Workers asked to help with one loop, at most.
const MAX_HELPERS: usize = 7;

/// How long the caller spins for the helpers still in a chunk before it
/// starts to yield its core. Yielding at once costs far more: when every
/// core is busy with background jobs, the system gives the core to one of
/// them, and the frame gets it back milliseconds later.
const SPIN_FOR: Duration = Duration::from_micros(500);

/// How a loop is cut and shared.
#[derive(Clone, Copy, Debug)]
pub struct Grain {
    /// Items a thread takes at a time: enough for about 10 µs of work,
    /// few enough for the threads to share the loop evenly.
    pub chunk: usize,
    /// One helper is asked for per this many chunks (the work that makes
    /// waking a thread worth it, 100 µs or so); a loop shorter than that
    /// runs on the calling thread alone.
    pub per_helper: usize,
}

impl Grain {
    fn chunks(self, items: usize) -> usize {
        items.div_ceil(self.chunk.max(1))
    }

    fn helpers(self, chunks: usize) -> usize {
        (chunks / self.per_helper.max(1)).min(MAX_HELPERS)
    }
}

/// Set in `Shared::state` once the caller has left the loop: no helper may
/// join any more.
const CLOSED: usize = 1 << (usize::BITS - 1);

/// A loop body with its lifetime erased, see `Shared::body`.
type Body<'a> = dyn Fn(usize) + Sync + 'a;

/// What the caller and its helpers share for one loop.
struct Shared {
    /// Next chunk to claim.
    next: AtomicUsize,
    chunks: usize,
    /// Helpers inside the loop, and `CLOSED`.
    state: AtomicUsize,
    /// The caller's loop body. It borrows from the caller's stack: only
    /// dereferenced by the caller, and by helpers counted in `state` (the
    /// caller does not return before it has set `CLOSED` and seen that
    /// count at zero).
    body: *const Body<'static>,
    /// Panic of a helper, raised again on the caller.
    panic: Mutex<Option<Box<dyn std::any::Any + Send>>>,
}

// SAFETY: `body` points to a `Sync` closure, called through a shared
// reference only, and only while the caller keeps it alive (see `body`);
// the other fields are `Send + Sync`.
unsafe impl Send for Shared {}
// SAFETY: as above.
unsafe impl Sync for Shared {}

impl Shared {
    /// Claim and run chunks until none is left.
    ///
    /// # Safety
    /// `body` must be alive: the caller of `for_chunks`, or a helper
    /// counted in `state`.
    unsafe fn work(&self) {
        // SAFETY: guaranteed by the caller of `work`.
        let body = unsafe { &*self.body };
        loop {
            // (at most one claim past the end per thread: no overflow)
            let chunk = self.next.fetch_add(1, Ordering::Relaxed);
            if chunk >= self.chunks {
                break;
            }
            body(chunk);
        }
    }

    /// A pool worker: joins the loop unless the caller has already left it.
    fn help(&self) {
        // (a helper the frame waits for: frame_priority.rs)
        aurora_render::frame_priority::apply();
        let mut state = self.state.load(Ordering::Acquire);
        loop {
            if state & CLOSED != 0 {
                return;
            }
            match self
                .state
                .compare_exchange_weak(state, state + 1, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => break,
                Err(s) => state = s,
            }
        }
        // SAFETY: counted in `state` before `CLOSED` was set, so the caller
        // waits in `close` until the count is given back below.
        let result = catch_unwind(AssertUnwindSafe(|| unsafe { self.work() }));
        if let Err(panic) = result {
            *self.panic.lock() = Some(panic);
        }
        self.state.fetch_sub(1, Ordering::Release);
    }

    /// The caller leaves the loop: no helper joins from now on, and those
    /// inside finish the chunk they hold (a few microseconds: spinning is
    /// shorter than a sleep and a wake-up).
    fn close(&self) {
        self.state.fetch_or(CLOSED, Ordering::AcqRel);
        let mut since: Option<Instant> = None;
        let mut spins = 0u32;
        while self.state.load(Ordering::Acquire) != CLOSED {
            spins += 1;
            std::hint::spin_loop();
            // (the clock is only read once the wait is no longer short)
            if spins.is_multiple_of(64) && since.get_or_insert_with(Instant::now).elapsed() > SPIN_FOR {
                // a helper lost its core in the middle of a chunk: it may
                // be waiting for this one
                std::thread::yield_now();
            }
        }
    }
}

/// Closes the loop when the caller leaves it, also by a panic of its body.
struct Close<'a>(&'a Shared);

impl Drop for Close<'_> {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// Run `body(chunk)` for every chunk of `0..chunks`, each once, on the
/// calling thread and on up to `helpers` workers of the pool that are free
/// to help.
fn for_chunks(chunks: usize, helpers: usize, body: impl Fn(usize) + Sync) {
    let helpers = helpers.min(chunks.saturating_sub(1)).min(rayon::current_num_threads());
    if helpers == 0 {
        (0..chunks).for_each(body);
        return;
    }
    let body: &Body<'_> = &body;
    // SAFETY: only the lifetime changes. The pointer is not used after this
    // function returns: `Close` below sets `CLOSED` and waits for the
    // helpers inside the loop before `body` goes out of scope, and a helper
    // that starts later does not touch it.
    let body: *const Body<'static> = unsafe { std::mem::transmute::<&Body<'_>, &'static Body<'static>>(body) };
    let shared = Arc::new(Shared {
        next: AtomicUsize::new(0),
        chunks,
        state: AtomicUsize::new(0),
        body,
        panic: Mutex::new(None),
    });
    for _ in 0..helpers {
        let shared = shared.clone();
        rayon::spawn(move || shared.help());
    }
    {
        let _close = Close(&shared);
        // SAFETY: `body` is alive, this is its owner's thread.
        unsafe { shared.work() };
    }
    if let Some(panic) = shared.panic.lock().take() {
        resume_unwind(panic);
    }
}

/// Fold `items` chunk by chunk: each chunk starts from `init()` and takes
/// its items in order. The results are in the order of the items.
pub fn fold<T: Sync, A: Send>(items: &[T], grain: Grain, init: impl Fn() -> A + Sync, step: impl Fn(A, &T) -> A + Sync) -> Vec<A> {
    let chunk = grain.chunk.max(1);
    let parts: Vec<Mutex<Option<A>>> = (0..grain.chunks(items.len())).map(|_| Mutex::new(None)).collect();
    for_chunks(parts.len(), grain.helpers(parts.len()), |i| {
        let acc = items[i * chunk..((i + 1) * chunk).min(items.len())].iter().fold(init(), &step);
        *parts[i].lock() = Some(acc);
    });
    parts.into_iter().filter_map(Mutex::into_inner).collect()
}

/// `f` on every item, which it may change; what it returns, in the order
/// of the items.
pub fn filter_map_mut<T: Send, R: Send>(items: &mut [T], grain: Grain, f: impl Fn(&mut T) -> Option<R> + Sync) -> Vec<R> {
    // (each chunk is locked once, by the thread that claimed it)
    let parts: Vec<Mutex<(&mut [T], Vec<R>)>> = items.chunks_mut(grain.chunk.max(1)).map(|c| Mutex::new((c, Vec::new()))).collect();
    for_chunks(parts.len(), grain.helpers(parts.len()), |i| {
        let (items, out) = &mut *parts[i].lock();
        out.extend(items.iter_mut().filter_map(&f));
    });
    parts.into_iter().flat_map(|p| p.into_inner().1).collect()
}

/// `f` on every item, which it may change.
pub fn for_each_mut<T: Send>(items: &mut [T], grain: Grain, f: impl Fn(&mut T) + Sync) {
    filter_map_mut(items, grain, |item| {
        f(item);
        None::<()>
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    const FINE: Grain = Grain { chunk: 1, per_helper: 1 };

    #[test]
    fn helpers_follow_the_work() {
        let g = Grain { chunk: 32, per_helper: 16 };
        assert_eq!(g.chunks(0), 0);
        assert_eq!(g.chunks(33), 2);
        // a short loop: the calling thread alone
        assert_eq!(g.helpers(g.chunks(480)), 0);
        assert_eq!(g.helpers(g.chunks(512)), 1);
        assert_eq!(g.helpers(g.chunks(2400)), 4);
        assert_eq!(g.helpers(g.chunks(100_000)), MAX_HELPERS);
        // never dividing by zero
        let odd = Grain { chunk: 0, per_helper: 0 };
        assert_eq!((odd.chunks(5), odd.helpers(5)), (5, 5));
    }

    #[test]
    fn every_chunk_runs_once() {
        for chunks in [0, 1, 2, 7, 64, 1000] {
            for helpers in [0, 1, 3, MAX_HELPERS] {
                let seen: Vec<AtomicU32> = (0..chunks).map(|_| AtomicU32::new(0)).collect();
                for_chunks(chunks, helpers, |i| {
                    seen[i].fetch_add(1, Ordering::Relaxed);
                });
                assert!(
                    seen.iter().all(|n| n.load(Ordering::Relaxed) == 1),
                    "{chunks} chunks, {helpers} helpers"
                );
            }
        }
    }

    #[test]
    fn fold_keeps_the_order_of_the_items() {
        let items: Vec<u32> = (0..1000).collect();
        for chunk in [1, 7, 32, 999, 1000, 5000] {
            let grain = Grain { chunk, per_helper: 2 };
            let parts = fold(&items, grain, Vec::new, |mut acc, &i| {
                acc.push(i * 2);
                acc
            });
            assert_eq!(parts.len(), items.len().div_ceil(chunk));
            let all: Vec<u32> = parts.into_iter().flatten().collect();
            assert_eq!(all, items.iter().map(|i| i * 2).collect::<Vec<_>>(), "chunks of {chunk}");
        }
        assert!(fold(&[] as &[u32], FINE, || 0u32, |acc, i| acc + i).is_empty());
    }

    #[test]
    fn items_are_changed_and_results_kept_in_order() {
        let mut items: Vec<u32> = (0..500).collect();
        for_each_mut(&mut items, Grain { chunk: 16, per_helper: 2 }, |i| *i += 1);
        assert_eq!(items, (1..=500).collect::<Vec<_>>());
        let even = filter_map_mut(&mut items, Grain { chunk: 7, per_helper: 1 }, |i| {
            *i *= 3;
            i.is_multiple_of(2).then_some(*i)
        });
        assert_eq!(items, (1..=500).map(|i| i * 3).collect::<Vec<_>>());
        assert_eq!(even, (1..=500).map(|i| i * 3).filter(|i| i % 2 == 0).collect::<Vec<_>>());
    }

    /// The loop is over when `for_chunks` returns, whatever the helpers
    /// do afterwards: what the body borrows can be changed right away.
    #[test]
    fn loops_in_a_row_borrow_the_same_data() {
        let mut total = 0u64;
        let mut local = [0u64; 64];
        for round in 0..2000u64 {
            local.fill(round);
            let sum = AtomicUsize::new(0);
            for_chunks(local.len(), MAX_HELPERS, |i| {
                sum.fetch_add(local[i] as usize, Ordering::Relaxed);
            });
            total += sum.into_inner() as u64;
        }
        assert_eq!(total, (0..2000u64).map(|r| r * 64).sum::<u64>());
    }

    #[test]
    fn a_panic_in_the_loop_reaches_the_caller() {
        let result = catch_unwind(|| {
            for_chunks(64, MAX_HELPERS, |i| {
                if i == 13 {
                    panic!("chunk 13");
                }
            })
        });
        assert!(result.is_err());
        // and the pool still works
        let n = AtomicUsize::new(0);
        for_chunks(64, MAX_HELPERS, |_| {
            n.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(n.into_inner(), 64);
    }
}
