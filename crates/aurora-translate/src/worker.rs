//! The translation thread: one model, one job at a time, the messages we
//! send before the ones we receive, the model freed after a quiet while.

use crate::lang::Lang;
use crate::prompt::{PromptKind, Rejected, clean_output, prompt};
use crossbeam_channel::{Receiver, Sender, select, unbounded};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Where the model runs once loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendInfo {
    pub on_gpu: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum TranslateError {
    #[error("translation is not built into this viewer")]
    NotBuilt,
    #[error("the translation model is not downloaded")]
    ModelMissing,
    #[error("the translation model did not load: {0}")]
    Load(String),
    #[error("the translation failed: {0}")]
    Generate(String),
    #[error("the model's answer was not a translation ({0:?})")]
    Rejected(Rejected),
    #[error("the translator stopped")]
    Closed,
}

/// A model runtime: llama.cpp, or the fake of the tests and the demo.
pub trait Backend: Send + 'static {
    fn load(&mut self) -> Result<BackendInfo, TranslateError>;
    fn generate(&mut self, prompt: &str, max_tokens: usize) -> Result<String, TranslateError>;
    fn unload(&mut self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Our message: someone waits for it, it goes first.
    Outgoing,
    Incoming,
}

#[derive(Debug)]
pub struct Job {
    pub id: u64,
    pub text: String,
    pub from: &'static Lang,
    pub to: &'static Lang,
    pub direction: Direction,
}

#[derive(Debug)]
pub struct Done {
    pub id: u64,
    pub result: Result<String, TranslateError>,
    pub millis: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Status {
    pub loaded: bool,
    pub on_gpu: bool,
    /// The job the model is working on (after the load).
    pub running: Option<u64>,
}

/// The handle of the translation thread. Dropping it stops the thread
/// (without waiting for it: the frame never blocks on the model).
pub struct Translator {
    outgoing: Sender<Job>,
    incoming: Sender<Job>,
    done: Receiver<Done>,
    status: Arc<Mutex<Status>>,
    thread: Option<JoinHandle<()>>,
}

impl Translator {
    pub fn spawn(backend: Box<dyn Backend>, kind: PromptKind, idle_unload: Duration) -> Translator {
        let (outgoing, out_rx) = unbounded();
        let (incoming, in_rx) = unbounded();
        let (done_tx, done) = unbounded();
        let status = Arc::new(Mutex::new(Status::default()));
        let w = Worker {
            backend,
            kind,
            idle_unload,
            status: status.clone(),
        };
        let thread = std::thread::Builder::new()
            .name("translator".into())
            .spawn(move || w.run(out_rx, in_rx, done_tx))
            .map_err(|e| log::error!("translator thread not started: {e}"))
            .ok();
        Translator {
            outgoing,
            incoming,
            done,
            status,
            thread,
        }
    }

    pub fn submit(&self, job: Job) {
        let queue = match job.direction {
            Direction::Outgoing => &self.outgoing,
            Direction::Incoming => &self.incoming,
        };
        if let Err(e) = queue.send(job) {
            log::warn!("translator gone, job {} dropped", e.0.id);
        }
    }

    pub fn try_recv(&self) -> Option<Done> {
        self.done.try_recv().ok()
    }

    pub fn status(&self) -> Status {
        self.status.lock().map(|s| *s).unwrap_or_default()
    }

    /// Whether the thread is gone (a panic in the backend).
    pub fn is_closed(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    #[cfg(test)]
    fn thread_for_test(&mut self) -> JoinHandle<()> {
        self.thread.take().expect("thread")
    }
}

struct Worker {
    backend: Box<dyn Backend>,
    kind: PromptKind,
    idle_unload: Duration,
    status: Arc<Mutex<Status>>,
}

impl Worker {
    fn run(mut self, out_rx: Receiver<Job>, in_rx: Receiver<Job>, done: Sender<Done>) {
        let mut held: VecDeque<Job> = VecDeque::new();
        let mut last = Instant::now();
        loop {
            let job = if let Ok(j) = out_rx.try_recv() {
                j
            } else if let Some(j) = held.pop_front() {
                j
            } else if let Ok(j) = in_rx.try_recv() {
                j
            } else {
                let wait = if self.loaded() {
                    self.idle_unload.saturating_sub(last.elapsed())
                } else {
                    Duration::from_secs(3600)
                };
                select! {
                    recv(out_rx) -> j => match j {
                        Ok(j) => j,
                        Err(_) => break,
                    },
                    recv(in_rx) -> j => match j {
                        // an outgoing job may have come at the same time:
                        // the loop's top takes it first
                        Ok(j) => {
                            held.push_back(j);
                            continue;
                        }
                        Err(_) => break,
                    },
                    default(wait) => {
                        if self.loaded() && last.elapsed() >= self.idle_unload {
                            log::info!("translation model unloaded after {:?} idle", self.idle_unload);
                            self.backend.unload();
                            self.set_status(Status::default());
                        }
                        continue;
                    }
                }
            };
            let start = Instant::now();
            let result = self.translate(&job);
            let millis = u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX);
            log::info!(
                "translation {} ({} chars) in {millis} ms, ok: {}",
                job.id,
                job.text.chars().count(),
                result.is_ok()
            );
            last = Instant::now();
            if done
                .send(Done {
                    id: job.id,
                    result,
                    millis,
                })
                .is_err()
            {
                break;
            }
        }
        self.backend.unload();
    }

    fn translate(&mut self, job: &Job) -> Result<String, TranslateError> {
        if !self.loaded() {
            let info = self.backend.load()?;
            self.set_status(Status {
                loaded: true,
                on_gpu: info.on_gpu,
                running: None,
            });
        }
        self.set_running(Some(job.id));
        let max_tokens = (4 * job.text.chars().count() + 32).min(400);
        let raw = self.backend.generate(&prompt(self.kind, &job.text, job.from, job.to), max_tokens);
        self.set_running(None);
        let raw = raw?;
        clean_output(&raw, &job.text).map_err(TranslateError::Rejected)
    }

    fn loaded(&self) -> bool {
        self.status.lock().map(|s| s.loaded).unwrap_or(false)
    }

    fn set_running(&self, id: Option<u64>) {
        if let Ok(mut g) = self.status.lock() {
            g.running = id;
        }
    }

    fn set_status(&self, s: Status) {
        if let Ok(mut g) = self.status.lock() {
            *g = s;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeBackend;
    use crate::lang::lang;
    use std::time::{Duration, Instant};

    fn job(id: u64, text: &str, direction: Direction) -> Job {
        Job {
            id,
            text: text.into(),
            from: lang("en").expect("en"),
            to: lang("es").expect("es"),
            direction,
        }
    }

    fn wait(t: &Translator, n: usize) -> Vec<Done> {
        let end = Instant::now() + Duration::from_secs(3);
        let mut out = Vec::new();
        while out.len() < n && Instant::now() < end {
            match t.try_recv() {
                Some(d) => out.push(d),
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        out
    }

    fn spawn(b: FakeBackend, idle: Duration) -> Translator {
        Translator::spawn(Box::new(b), PromptKind::TranslateGemma, idle)
    }

    #[test]
    fn translates_a_job() {
        let t = spawn(FakeBackend::new(), Duration::from_secs(60));
        t.submit(job(7, "hello", Direction::Incoming));
        let done = wait(&t, 1);
        assert_eq!(done[0].id, 7);
        assert_eq!(done[0].result.as_deref().ok(), Some("[es] hello"));
        assert!(t.status().loaded);
    }

    #[test]
    fn outgoing_jumps_the_queue() {
        let t = spawn(FakeBackend::slow(Duration::from_millis(100)), Duration::from_secs(60));
        t.submit(job(1, "one", Direction::Incoming));
        std::thread::sleep(Duration::from_millis(30));
        t.submit(job(2, "two", Direction::Incoming));
        t.submit(job(3, "three", Direction::Outgoing));
        let order: Vec<u64> = wait(&t, 3).iter().map(|d| d.id).collect();
        assert_eq!(order, [1, 3, 2]);
    }

    #[test]
    fn unloads_when_idle() {
        let t = spawn(FakeBackend::new(), Duration::from_millis(50));
        t.submit(job(1, "hello", Direction::Incoming));
        wait(&t, 1);
        std::thread::sleep(Duration::from_millis(250));
        assert!(!t.status().loaded);
    }

    #[test]
    fn errors_come_back() {
        let t = spawn(FakeBackend::failing(), Duration::from_secs(60));
        t.submit(job(1, "hello", Direction::Outgoing));
        assert!(matches!(wait(&t, 1)[0].result, Err(TranslateError::Generate(_))));
    }

    #[test]
    fn drop_stops_the_thread() {
        let mut t = spawn(FakeBackend::new(), Duration::from_secs(60));
        let handle = t.thread_for_test();
        let start = Instant::now();
        drop(t);
        while !handle.is_finished() && start.elapsed() < Duration::from_secs(1) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(handle.is_finished());
    }
}
