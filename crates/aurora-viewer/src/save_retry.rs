//! A save that a refused write does not lose: the change stays owed, it is
//! tried again a little later, then less and less often, and once more when
//! the viewer closes. The rule of the omnifilter's writer (`omnifilter.rs`,
//! `write_failed`), for the files written by a thread of their own.
//!
//! The Legacy Viewer writes such files once and keeps the change in memory:
//! LLFavoritesOrderStorage::saveFavoritesRecord leaves `mSaveOnExit` set when
//! `stored_favorites.xml` does not open, and destroyClass writes it again at
//! exit (indra/newview/llfavoritesbar.cpp). Here the change is also tried
//! again during the session, so a file held for a moment by another program
//! (an antivirus, the indexer) or a folder that refuses writes for a while
//! does not wait for the exit.

use crossbeam_channel::{RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The wait before the first try again: the omnifilter's 5 s.
const FIRST_WAIT: Duration = Duration::from_secs(5);
/// The longest wait between two tries of a save that keeps failing.
const LONGEST_WAIT: Duration = Duration::from_secs(300);

/// When a save that failed is tried again: after `first`, then twice as
/// long after each new failure, `longest` at most.
#[derive(Clone, Copy, Debug)]
pub struct Backoff {
    first: Duration,
    longest: Duration,
    wait: Duration,
    next: Option<Instant>,
}

impl Default for Backoff {
    fn default() -> Self {
        Backoff::new(FIRST_WAIT, LONGEST_WAIT)
    }
}

impl Backoff {
    pub fn new(first: Duration, longest: Duration) -> Self {
        Backoff {
            first,
            longest: longest.max(first),
            wait: Duration::ZERO,
            next: None,
        }
    }

    /// A write did not reach the disk: the next try waits longer.
    pub fn failed(&mut self, now: Instant) -> Duration {
        self.wait = (self.wait * 2).clamp(self.first, self.longest);
        self.next = Some(now + self.wait);
        self.wait
    }

    /// A write reached the disk: the next failure waits `first` again.
    pub fn succeeded(&mut self) {
        self.wait = Duration::ZERO;
        self.next = None;
    }

    /// A write may be made now: none failed, or the wait is over.
    pub fn due(&self, now: Instant) -> bool {
        self.next.is_none_or(|t| now >= t)
    }

    /// The time left before the next try (zero when it is due).
    pub fn left(&self, now: Instant) -> Duration {
        self.next.map_or(Duration::ZERO, |t| t.saturating_duration_since(now))
    }
}

/// A thread that runs a save each time it is asked to, and again, waiting
/// longer each time, as long as it fails. The save reads what it has to
/// write from its owner's state (a queue of changes, a snapshot): it is
/// asked once per change, and a run writes every change owed so far.
/// Dropped (the viewer closing, the account logging out), it runs the save
/// once more if something is still owed, and waits for that run.
pub struct Retrying {
    asks: Option<Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl Retrying {
    /// `what` names the file in the log.
    pub fn start(name: &str, what: &'static str, save: impl FnMut() -> std::io::Result<()> + Send + 'static) -> Option<Self> {
        Self::start_with(name, what, Backoff::default(), save)
    }

    fn start_with(
        name: &str,
        what: &'static str,
        mut backoff: Backoff,
        mut save: impl FnMut() -> std::io::Result<()> + Send + 'static,
    ) -> Option<Self> {
        let (asks, asked) = crossbeam_channel::unbounded::<()>();
        let worker = std::thread::Builder::new().name(name.into()).spawn(move || {
            let mut owed = false;
            loop {
                let got = if owed {
                    match asked.recv_timeout(backoff.left(Instant::now())) {
                        Ok(()) | Err(RecvTimeoutError::Timeout) => true,
                        Err(RecvTimeoutError::Disconnected) => false,
                    }
                } else {
                    asked.recv().is_ok()
                };
                if !got {
                    // the owner is gone: the last try, as at exit
                    if owed && let Err(error) = save() {
                        log::warn!("{what} not saved at exit: {error}");
                    }
                    return;
                }
                asked.try_iter().for_each(drop);
                owed = true;
                let now = Instant::now();
                if !backoff.due(now) {
                    continue;
                }
                match save() {
                    Ok(()) => {
                        owed = false;
                        backoff.succeeded();
                    }
                    Err(error) => {
                        let wait = backoff.failed(now);
                        log::warn!("{what} not saved ({error}); tried again in {} s", wait.as_secs());
                    }
                }
            }
        });
        match worker {
            Ok(worker) => Some(Retrying {
                asks: Some(asks),
                worker: Some(worker),
            }),
            Err(error) => {
                log::warn!("cannot start the writer of the {what}: {error}");
                None
            }
        }
    }

    /// Something changed: save it now, or at the next try if the last
    /// one failed a moment ago.
    pub fn ask(&self) {
        if let Some(asks) = &self.asks {
            let _ = asks.send(());
        }
    }
}

impl Drop for Retrying {
    fn drop(&mut self) {
        self.asks.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The loop of a writer thread that saves whole states (a list, a map):
/// the newest one received is written. A refused one stays owed and is
/// tried again, waiting longer each time ([`Backoff`]); a newer one that
/// comes meanwhile takes its place and waits for the same try. When the
/// channel closes (its owner dropped: a logout, the exit) the state still
/// owed is tried once more, then the loop returns.
pub fn write_latest<T>(what: &'static str, received: &crossbeam_channel::Receiver<T>, save: impl FnMut(&T) -> std::io::Result<()>) {
    write_latest_with(what, received, Backoff::default(), save);
}

fn write_latest_with<T>(
    what: &'static str,
    received: &crossbeam_channel::Receiver<T>,
    mut backoff: Backoff,
    mut save: impl FnMut(&T) -> std::io::Result<()>,
) {
    let mut owed: Option<T> = None;
    loop {
        let next = match &owed {
            None => match received.recv() {
                Ok(state) => Some(state),
                Err(_) => return,
            },
            Some(state) => match received.recv_timeout(backoff.left(Instant::now())) {
                Ok(newer) => Some(newer),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => {
                    if let Err(error) = save(state) {
                        log::warn!("{what} not saved at exit: {error}");
                    }
                    return;
                }
            },
        };
        if let Some(mut state) = next {
            while let Ok(newer) = received.try_recv() {
                state = newer;
            }
            owed = Some(state);
        }
        let now = Instant::now();
        let Some(state) = owed.as_ref().filter(|_| backoff.due(now)) else {
            continue;
        };
        match save(state) {
            Ok(()) => {
                owed = None;
                backoff.succeeded();
            }
            Err(error) => {
                let wait = backoff.failed(now);
                log::warn!("{what} not saved ({error}); tried again in {} s", wait.as_secs());
            }
        }
    }
}

/// The [`Retrying`] of a file whose owed changes a `static` keeps (the
/// account list, the word lists): started at the first refused write, and
/// ended with its last try by [`OnDemand::finish`] when the viewer closes
/// (a `static` is never dropped).
pub struct OnDemand {
    name: &'static str,
    what: &'static str,
    retrying: parking_lot::Mutex<Option<Retrying>>,
}

impl OnDemand {
    pub const fn new(name: &'static str, what: &'static str) -> Self {
        OnDemand {
            name,
            what,
            retrying: parking_lot::Mutex::new(None),
        }
    }

    /// A write was refused: `save` writes what is owed, now on its thread
    /// and again until it goes through (the first `save` given stays).
    pub fn ask(&self, save: impl FnMut() -> std::io::Result<()> + Send + 'static) {
        let mut retrying = self.retrying.lock();
        if retrying.is_none() {
            *retrying = Retrying::start(self.name, self.what, save);
        }
        if let Some(retrying) = retrying.as_ref() {
            retrying.ask();
        }
    }

    /// The viewer closing: what is still owed is tried once more, and
    /// waited for.
    pub fn finish(&self) {
        let retrying = self.retrying.lock().take();
        drop(retrying);
    }
}

/// Whole files written by the frame or a caller's thread, whose refused
/// write keeps its bytes, the latest per file: tried again by a thread
/// ([`OnDemand`]) and once more by [`OwedFiles::finish`] at exit. A later
/// write of the same file that goes through drops the older bytes owed.
pub struct OwedFiles {
    files: parking_lot::Mutex<Vec<(std::path::PathBuf, Vec<u8>)>>,
    retry: OnDemand,
}

impl OwedFiles {
    pub const fn new(name: &'static str, what: &'static str) -> Self {
        OwedFiles {
            files: parking_lot::Mutex::new(Vec::new()),
            retry: OnDemand::new(name, what),
        }
    }

    /// `bytes` written as `path` now (its folder made if needed), or kept
    /// to be written later: the error says it was refused, and kept.
    pub fn write(&'static self, path: &std::path::Path, bytes: Vec<u8>) -> std::io::Result<()> {
        let mut files = self.files.lock();
        files.retain(|(p, _)| p != path);
        files.push((path.to_owned(), bytes));
        let written = Self::write_all(&mut files);
        drop(files);
        if written.is_err() {
            self.retry.ask(move || Self::write_all(&mut self.files.lock()));
        }
        written
    }

    /// The bytes still owed for `path`: what a read of it should see.
    pub fn owed(&self, path: &std::path::Path) -> Option<Vec<u8>> {
        self.files.lock().iter().find(|(p, _)| p == path).map(|(_, b)| b.clone())
    }

    /// The viewer closing: what is still owed is tried once more.
    pub fn finish(&self) {
        self.retry.finish();
    }

    /// Each owed file written; the ones refused stay owed.
    fn write_all(files: &mut Vec<(std::path::PathBuf, Vec<u8>)>) -> std::io::Result<()> {
        let mut refused = None;
        files.retain(|(path, bytes)| {
            let written = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| crate::atomic_file::write(path, bytes));
            match written {
                Ok(()) => false,
                Err(error) => {
                    refused.get_or_insert(std::io::Error::new(error.kind(), format!("{}: {error}", path.display())));
                    true
                }
            }
        });
        refused.map_or(Ok(()), Err)
    }

    /// The retry's run, at once (the tests).
    #[cfg(test)]
    pub fn retry_now(&self) {
        let _ = Self::write_all(&mut self.files.lock());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn the_wait_doubles_up_to_the_longest_and_starts_again_after_a_success() {
        let mut b = Backoff::default();
        let t = Instant::now();
        assert!(b.due(t));
        assert_eq!(b.failed(t), Duration::from_secs(5));
        assert!(!b.due(t + Duration::from_secs(4)));
        assert!(b.due(t + Duration::from_secs(5)));
        assert_eq!(b.failed(t), Duration::from_secs(10));
        for _ in 0..10 {
            b.failed(t);
        }
        assert_eq!(b.failed(t), Duration::from_secs(300));
        assert_eq!(b.left(t), Duration::from_secs(300));
        b.succeeded();
        assert!(b.due(t));
        assert_eq!(b.failed(t), Duration::from_secs(5));
    }

    /// A save that is refused twice then goes through: tried again by
    /// itself, without another change asking for it.
    #[test]
    fn a_refused_save_is_tried_again_until_it_goes_through() {
        let runs = Arc::new(AtomicUsize::new(0));
        let counted = runs.clone();
        let retrying = Retrying::start_with(
            "aurora-test-retry",
            "test file",
            Backoff::new(Duration::from_millis(5), Duration::from_millis(20)),
            move || match counted.fetch_add(1, Ordering::SeqCst) {
                0 | 1 => Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
                _ => Ok(()),
            },
        )
        .expect("thread");
        retrying.ask();
        let deadline = Instant::now() + Duration::from_secs(10);
        while runs.load(Ordering::SeqCst) < 3 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        // gone through: nothing owed, no try at exit
        drop(retrying);
        assert_eq!(runs.load(Ordering::SeqCst), 3);
    }

    /// A state refused twice then written, a newer state sent during the
    /// wait taking its place: the newest is what reaches the disk.
    #[test]
    fn the_latest_state_is_written_once_the_refusals_end() {
        let (send, received) = crossbeam_channel::unbounded::<u32>();
        let written = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let disk = written.clone();
        let tries = Arc::new(AtomicUsize::new(0));
        let counted = tries.clone();
        let worker = std::thread::spawn(move || {
            write_latest_with(
                "test state",
                &received,
                Backoff::new(Duration::from_millis(5), Duration::from_millis(20)),
                move |state| {
                    if counted.fetch_add(1, Ordering::SeqCst) < 2 {
                        return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
                    }
                    disk.lock().push(*state);
                    Ok(())
                },
            )
        });
        send.send(1).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while tries.load(Ordering::SeqCst) < 1 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        send.send(2).unwrap();
        while written.lock().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(send);
        worker.join().unwrap();
        assert_eq!(*written.lock(), [2]);
    }

    /// A state still refused when the channel closes is tried once more.
    #[test]
    fn a_state_still_owed_is_tried_once_more_when_the_channel_closes() {
        let (send, received) = crossbeam_channel::unbounded::<u32>();
        let tries = Arc::new(AtomicUsize::new(0));
        let counted = tries.clone();
        let worker = std::thread::spawn(move || {
            write_latest_with(
                "test state",
                &received,
                Backoff::new(Duration::from_secs(60), Duration::from_secs(60)),
                move |_| {
                    counted.fetch_add(1, Ordering::SeqCst);
                    Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
                },
            )
        });
        send.send(1).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while tries.load(Ordering::SeqCst) < 1 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        drop(send);
        worker.join().unwrap();
        assert_eq!(tries.load(Ordering::SeqCst), 2);
    }

    /// A file whose folder refuses writes (a file stands in its place): its
    /// bytes are kept and read back, a retry writes them once the folder
    /// is free, nothing owed then.
    #[test]
    fn an_owed_file_is_kept_read_back_and_written_later() {
        static FILES: OwedFiles = OwedFiles::new("aurora-test-owed", "test file");
        let root = std::env::temp_dir().join(format!("aurora-owed-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let folder = root.join("account");
        std::fs::write(&folder, b"in the way").unwrap();
        let path = folder.join("state.json");
        assert!(FILES.write(&path, b"old".to_vec()).is_err());
        assert!(FILES.write(&path, b"new".to_vec()).is_err());
        assert_eq!(FILES.owed(&path).as_deref(), Some(&b"new"[..]), "the latest only");
        std::fs::remove_file(&folder).unwrap();
        FILES.retry_now();
        assert_eq!(FILES.owed(&path), None);
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        FILES.finish();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A save still refused when the owner goes is tried once more then.
    #[test]
    fn what_is_still_owed_is_tried_once_more_at_exit() {
        let runs = Arc::new(AtomicUsize::new(0));
        let counted = runs.clone();
        let retrying = Retrying::start_with(
            "aurora-test-retry-exit",
            "test file",
            Backoff::new(Duration::from_secs(60), Duration::from_secs(60)),
            move || {
                counted.fetch_add(1, Ordering::SeqCst);
                Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
            },
        )
        .expect("thread");
        retrying.ask();
        let deadline = Instant::now() + Duration::from_secs(10);
        while runs.load(Ordering::SeqCst) < 1 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        // a change during the wait waits for it too
        retrying.ask();
        drop(retrying);
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }
}
