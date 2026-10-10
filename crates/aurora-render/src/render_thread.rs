//! The render thread and the hand-off of frames to it.
//!
//! The main thread builds a frame packet (packet.rs) and goes on with the
//! next frame; this thread turns the packet into GPU work: queue writes,
//! pass encoding, `finish`, `submit`, `present`. A frame then costs the
//! longer of the two, not their sum.
//!
//! The hand-off ([`handoff`]) is a rendezvous, not a queue: the worker
//! holds at most one job and nothing waits behind it. When the main thread
//! comes with frame N+1 while frame N is still being drawn, it waits
//! (back-pressure) until N is presented, then hands N+1 over. A queued
//! frame would only add a frame of latency: the rate is bounded by the
//! slower thread either way. Each side measures its wait, reported by the
//! frame profile.
//!
//! Nothing here can block for ever: when the worker goes away (it
//! returned, or it panicked and its guard is dropped), every wait of the
//! owner ends with [`WorkerGone`]; when the owner closes or is dropped, the
//! worker's wait ends and it leaves its loop.
//!
//! AURORA_RENDER_THREAD=0 keeps the renderer on the main thread
//! ([`Host::Inline`]): the same packet, executed in place.

use crate::packet::{FramePacket, FrameResult};
use crate::renderer::Backend;
use glam::Vec3;
use parking_lot::{Condvar, Mutex};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The worker of a hand-off is gone: it returned or panicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerGone;

struct State<J, R> {
    /// Handed over, not taken by the worker yet.
    job: Option<J>,
    /// The worker is on a job it took.
    busy: bool,
    /// Result of the last finished job, until the owner takes it.
    result: Option<R>,
    /// The owner asked the worker to stop.
    closed: bool,
    worker_gone: bool,
}

struct Shared<J, R> {
    state: Mutex<State<J, R>>,
    to_worker: Condvar,
    to_owner: Condvar,
}

/// A rendezvous between the thread that makes jobs (owner) and the one
/// that runs them (worker): one job at a time, none queued.
pub fn handoff<J, R>() -> (Owner<J, R>, Worker<J, R>) {
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            job: None,
            busy: false,
            result: None,
            closed: false,
            worker_gone: false,
        }),
        to_worker: Condvar::new(),
        to_owner: Condvar::new(),
    });
    (Owner(shared.clone()), Worker(shared))
}

/// What a `submit` returns.
#[derive(Debug, PartialEq)]
pub struct Submitted<R> {
    /// Result of the job before this one, when it was not taken yet.
    pub previous: Option<R>,
    /// Time spent waiting for the worker to be free (back-pressure).
    pub waited: Duration,
}

pub struct Owner<J, R>(Arc<Shared<J, R>>);

impl<J, R> Owner<J, R> {
    /// Wait until the worker holds nothing; the time it took.
    fn wait_free(&self, s: &mut parking_lot::MutexGuard<'_, State<J, R>>) -> Result<Duration, WorkerGone> {
        let mut waited = Duration::ZERO;
        if (s.busy || s.job.is_some()) && !s.worker_gone {
            let t = Instant::now();
            while (s.busy || s.job.is_some()) && !s.worker_gone {
                self.0.to_owner.wait(s);
            }
            waited = t.elapsed();
        }
        if s.worker_gone { Err(WorkerGone) } else { Ok(waited) }
    }

    /// Hand a job over, waiting first for the worker to finish the one it
    /// has. Returns as soon as the job is handed over.
    pub fn submit(&self, job: J) -> Result<Submitted<R>, WorkerGone> {
        let mut s = self.0.state.lock();
        let waited = self.wait_free(&mut s)?;
        let previous = s.result.take();
        s.job = Some(job);
        self.0.to_worker.notify_one();
        Ok(Submitted { previous, waited })
    }

    /// Wait for the job handed over to be done; its result unless already
    /// taken (None also when nothing was submitted).
    pub fn finish(&self) -> Result<Option<R>, WorkerGone> {
        let mut s = self.0.state.lock();
        self.wait_free(&mut s)?;
        Ok(s.result.take())
    }

    /// Ask the worker to stop: its wait for a job ends, a job not taken
    /// yet is dropped.
    pub fn close(&self) {
        let mut s = self.0.state.lock();
        s.closed = true;
        s.job = None;
        self.0.to_worker.notify_all();
    }
}

impl<J, R> Drop for Owner<J, R> {
    fn drop(&mut self) {
        self.close();
    }
}

pub struct Worker<J, R>(Arc<Shared<J, R>>);

impl<J, R> Worker<J, R> {
    /// Wait for the next job; the time spent waiting for it. None once
    /// the owner closed.
    pub fn next(&self) -> Option<(J, Duration)> {
        let mut s = self.0.state.lock();
        let t = Instant::now();
        while s.job.is_none() && !s.closed {
            self.0.to_worker.wait(&mut s);
        }
        if s.closed {
            return None;
        }
        let job = s.job.take()?;
        s.busy = true;
        Some((job, t.elapsed()))
    }

    /// The job taken is done: the owner may hand the next one over.
    pub fn complete(&self, result: R) {
        let mut s = self.0.state.lock();
        s.result = Some(result);
        s.busy = false;
        self.0.to_owner.notify_all();
    }
}

impl<J, R> Drop for Worker<J, R> {
    /// Also runs when the worker thread unwinds from a panic: the owner
    /// must never wait for a thread that is gone.
    fn drop(&mut self) {
        let mut s = self.0.state.lock();
        s.worker_gone = true;
        s.busy = false;
        s.job = None;
        self.0.to_owner.notify_all();
    }
}

/// What the render thread is asked to do.
pub(crate) enum Job {
    Frame(Box<FramePacket>),
    /// Depth under a pixel of the last frame drawn, waiting for the GPU
    /// (clicks). Handed over like a frame, so it runs after the frame in
    /// flight: the depth and the camera are those of the last frame built.
    Pick {
        x: f32,
        y: f32,
    },
}

pub(crate) enum Done {
    Frame(Box<FrameResult>),
    Pick(Option<Vec3>),
}

/// Where the backend runs.
pub(crate) enum Host {
    /// On the calling thread (AURORA_RENDER_THREAD=0).
    Inline(Box<Backend>),
    Thread {
        owner: Owner<Job, Done>,
        /// Gives the backend back when it ends.
        thread: Option<std::thread::JoinHandle<Option<Backend>>>,
    },
}

/// AURORA_RENDER_THREAD=0 turns the render thread off (comparison,
/// debugging); anything else, or nothing, leaves it on.
pub(crate) fn thread_wanted(var: Option<&str>) -> bool {
    !matches!(var.map(str::trim), Some("0" | "off" | "false" | "no"))
}

impl Host {
    pub fn new(backend: Backend, threaded: bool) -> Host {
        if !threaded {
            return Host::Inline(Box::new(backend));
        }
        let (owner, worker) = handoff();
        // the backend goes to the thread; it comes back only if the thread
        // cannot be started
        let slot = Arc::new(Mutex::new(Some(backend)));
        let for_thread = slot.clone();
        let spawned = std::thread::Builder::new().name("aurora-render".into()).spawn(move || {
            let backend = for_thread.lock().take();
            backend.map(|backend| run(backend, worker))
        });
        match spawned {
            Ok(thread) => Host::Thread {
                owner,
                thread: Some(thread),
            },
            Err(e) => {
                log::warn!("render thread not started ({e}): rendering on the main thread");
                match slot.lock().take() {
                    Some(backend) => Host::Inline(Box::new(backend)),
                    // the closure never ran, so the backend is still there;
                    // were it not, every hand-over would report the worker gone
                    None => Host::Thread { owner, thread: None },
                }
            }
        }
    }

    pub fn threaded(&self) -> bool {
        matches!(self, Host::Thread { .. })
    }
}

impl Drop for Host {
    /// The render thread finishes the frame in its hands, then leaves and
    /// gives the backend back: the surface and the swapchain are destroyed
    /// by the thread that created them, as without a render thread.
    fn drop(&mut self) {
        if let Host::Thread { owner, thread } = self {
            owner.close();
            if let Some(t) = thread.take() {
                match t.join() {
                    Ok(backend) => drop(backend),
                    Err(_) => log::error!("render thread ended by a panic"),
                }
            }
        }
    }
}

/// The render thread's loop; returns the backend when the owner closes.
fn run(mut backend: Backend, worker: Worker<Job, Done>) -> Backend {
    while let Some((job, idle)) = worker.next() {
        let done = match job {
            Job::Frame(packet) => {
                let t = Instant::now();
                let mut result = backend.render(*packet);
                result.stats.thread_ms = t.elapsed().as_secs_f32() * 1000.0;
                result.stats.thread_idle_ms = idle.as_secs_f32() * 1000.0;
                Done::Frame(Box::new(result))
            }
            Job::Pick { x, y } => Done::Pick(backend.pick_world(x, y)),
        };
        worker.complete(done);
    }
    backend
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc;

    const TICK: Duration = Duration::from_millis(40);

    #[test]
    fn the_switch_only_turns_the_thread_off_explicitly() {
        assert!(thread_wanted(None));
        assert!(thread_wanted(Some("1")));
        assert!(thread_wanted(Some("")));
        assert!(!thread_wanted(Some("0")));
        assert!(!thread_wanted(Some(" off ")));
        assert!(!thread_wanted(Some("false")));
    }

    #[test]
    fn results_come_back_in_order_one_frame_late() {
        let (owner, worker) = handoff::<u32, u32>();
        let t = std::thread::spawn(move || {
            while let Some((job, _)) = worker.next() {
                worker.complete(job * 10);
            }
        });
        // the first hand-over has no previous result; each next one brings
        // the result of the job before it
        assert_eq!(owner.submit(1).map(|s| s.previous), Ok(None));
        assert_eq!(owner.submit(2).map(|s| s.previous), Ok(Some(10)));
        assert_eq!(owner.submit(3).map(|s| s.previous), Ok(Some(20)));
        // a synchronous job (capture, pick): its own result
        assert_eq!(owner.finish(), Ok(Some(30)));
        // already taken
        assert_eq!(owner.finish(), Ok(None));
        assert_eq!(owner.submit(4).map(|s| s.previous), Ok(None));
        drop(owner);
        t.join().expect("worker");
    }

    #[test]
    fn the_owner_waits_while_the_worker_is_busy() {
        let (owner, worker) = handoff::<u32, u32>();
        let (release, gate) = mpsc::channel::<()>();
        let in_hand = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));
        let (ih, mo) = (in_hand.clone(), most.clone());
        let t = std::thread::spawn(move || {
            while let Some((job, _)) = worker.next() {
                let n = ih.fetch_add(1, Ordering::SeqCst) + 1;
                mo.fetch_max(n, Ordering::SeqCst);
                // a slow frame: held until the test lets it go
                let _ = gate.recv();
                ih.fetch_sub(1, Ordering::SeqCst);
                worker.complete(job);
            }
        });
        // frame 1 is handed over at once
        let first = owner.submit(1).expect("worker alive");
        assert!(first.waited < TICK, "{first:?}");
        // frame 2 must wait for frame 1: nothing is queued behind it
        let owner = Arc::new(owner);
        let handed = Arc::new(AtomicBool::new(false));
        let (o2, h2) = (owner.clone(), handed.clone());
        let second = std::thread::spawn(move || {
            let s = o2.submit(2).expect("worker alive");
            h2.store(true, Ordering::SeqCst);
            s
        });
        std::thread::sleep(TICK);
        assert!(!handed.load(Ordering::SeqCst), "frame 2 handed over while frame 1 is being drawn");
        release.send(()).expect("worker waiting");
        let s = second.join().expect("second submit");
        assert_eq!(s.previous, Some(1));
        assert!(s.waited >= TICK / 2, "back-pressure not measured: {s:?}");
        release.send(()).expect("worker waiting");
        assert_eq!(owner.finish(), Ok(Some(2)));
        assert_eq!(most.load(Ordering::SeqCst), 1, "the worker held two jobs");
        drop(release);
        drop(owner);
        t.join().expect("worker");
    }

    #[test]
    fn the_worker_measures_its_wait_for_the_owner() {
        let (owner, worker) = handoff::<u32, Duration>();
        let t = std::thread::spawn(move || {
            while let Some((_, idle)) = worker.next() {
                worker.complete(idle);
            }
        });
        std::thread::sleep(TICK);
        owner.submit(1).expect("worker alive");
        let idle = owner.finish().expect("worker alive").expect("result");
        assert!(idle >= TICK / 2, "{idle:?}");
        drop(owner);
        t.join().expect("worker");
    }

    #[test]
    fn a_dead_worker_never_blocks_the_owner() {
        let (owner, worker) = handoff::<u32, u32>();
        let t = std::thread::spawn(move || {
            let (job, _) = worker.next().expect("job");
            // the frame fails for good (device lost, a bug): the thread
            // unwinds with the job in its hands
            if job == 1 {
                std::panic::resume_unwind(Box::new("render thread down"));
            }
            worker.complete(job);
        });
        owner.submit(1).expect("handed over");
        assert!(t.join().is_err());
        // waiting for the frame, or handing the next one over, ends at once
        assert_eq!(owner.finish(), Err(WorkerGone));
        assert_eq!(owner.submit(2).map(|s| s.previous), Err(WorkerGone));
    }

    #[test]
    fn a_worker_that_leaves_while_the_owner_waits_wakes_it() {
        let (owner, worker) = handoff::<u32, u32>();
        let t = std::thread::spawn(move || {
            let _job = worker.next();
            std::thread::sleep(TICK);
            // returns without completing: `worker` is dropped
        });
        owner.submit(1).expect("handed over");
        assert_eq!(owner.finish(), Err(WorkerGone));
        t.join().expect("worker");
    }

    #[test]
    fn closing_ends_the_worker_loop() {
        let (owner, worker) = handoff::<u32, u32>();
        let done = Arc::new(AtomicUsize::new(0));
        let d = done.clone();
        let t = std::thread::spawn(move || {
            while let Some((job, _)) = worker.next() {
                d.fetch_add(1, Ordering::SeqCst);
                worker.complete(job);
            }
        });
        owner.submit(1).expect("worker alive");
        assert_eq!(owner.finish(), Ok(Some(1)));
        // the window closes (during a login, a teleport: whenever): the
        // worker, idle or not, leaves
        drop(owner);
        t.join().expect("worker");
        assert_eq!(done.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn closing_while_a_job_is_in_hand_lets_it_finish() {
        let (owner, worker) = handoff::<u32, u32>();
        let (release, gate) = mpsc::channel::<()>();
        let finished = Arc::new(AtomicBool::new(false));
        let f = finished.clone();
        let t = std::thread::spawn(move || {
            while let Some((job, _)) = worker.next() {
                let _ = gate.recv();
                f.store(true, Ordering::SeqCst);
                worker.complete(job);
            }
        });
        owner.submit(1).expect("worker alive");
        std::thread::sleep(TICK);
        owner.close();
        release.send(()).expect("worker on its job");
        t.join().expect("worker");
        assert!(finished.load(Ordering::SeqCst));
    }
}
