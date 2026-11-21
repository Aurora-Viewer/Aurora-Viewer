//! The user's files written off the frame thread. A write flushes the file
//! to the disk (atomic_file.rs, #1809), a few milliseconds and more on a
//! busy disk, and waits up to 2 s on Windows while another program holds
//! the file: on the frame thread it stalls the display, and at exit it held
//! a closing viewer frozen inside the inventory cache's write for over a
//! minute (#3466). The callers hand the write here and go on:
//!
//! - the saves of one file are written in the order they were asked, by one
//!   thread at a time, so an older state never lands after a newer one;
//! - [`replace`]: a save that writes the whole file drops the saves of that
//!   file not started yet, which held an older state of it;
//! - [`change`]: a save that changes the file from what it holds (read,
//!   change, write) runs after every save of the file asked before it;
//! - each file with saves waiting has a thread of its own, which ends once
//!   they are written: a file held for 2 s, or a large cache file, never
//!   holds the saves of the others back;
//! - [`wait`] before the viewer ends, [`wait_for`] before a file is read
//!   back, each with a bound: a disk that does not answer never keeps the
//!   viewer from closing.
//!
//! A save reports its own failure (a log line, or a message it hands back
//! to the frame through a channel of its own): the queue only orders them.
//! Aurora's own: the Legacy Viewer writes these files on its main thread
//! (LLControlGroup::saveToFile, indra/llxml/llcontrol.cpp;
//! LLInventoryModel::saveToFile, indra/newview/llinventorymodel.cpp). The
//! files and what they hold are the same.
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

type Job = Box<dyn FnOnce() + Send>;

/// How long a read of a file waits for the saves of it still under way
/// ([`wait_for`]): a flush and the 2 s a held file may take on Windows
/// (atomic_file.rs). Nothing is waited for when the file has none, the
/// usual case: the saves of a logout are long written when the next login
/// reads the files back.
pub const BEFORE_READ: Duration = Duration::from_secs(3);

#[derive(Default)]
struct State {
    /// The files with saves left, each the saves not started yet. A file is
    /// listed for as long as its thread runs: until its last save is
    /// written.
    files: HashMap<PathBuf, VecDeque<Job>>,
}

#[derive(Default)]
struct Saves {
    state: Mutex<State>,
    /// A file's last save was written.
    done: Condvar,
}

/// Write the whole file at `path` with `job`, after its saves under way;
/// its saves not started yet are dropped.
pub fn replace(path: impl Into<PathBuf>, job: impl FnOnce() + Send + 'static) {
    queue(saves(), path.into(), true, Box::new(job));
}

/// Change the file at `path` with `job`, after every save of it asked
/// before.
pub fn change(path: impl Into<PathBuf>, job: impl FnOnce() + Send + 'static) {
    queue(saves(), path.into(), false, Box::new(job));
}

/// Until every save asked is written, `timeout` at most: before the viewer
/// ends. The files left are logged.
pub fn wait(timeout: Duration) -> bool {
    let left = saves().wait(None, timeout);
    if !left.is_empty() {
        log::warn!("saves: not written within {timeout:?}: {}", names(&left));
    }
    left.is_empty()
}

/// Until `path` has no save left, `timeout` at most: before it is read
/// back. Nothing to wait for when it has none. Never from one of its own
/// saves, which would wait for itself.
pub fn wait_for(path: &Path, timeout: Duration) -> bool {
    let left = saves().wait(Some(path), timeout);
    if !left.is_empty() {
        log::warn!("saves: {} read before its save was written", names(&left));
    }
    left.is_empty()
}

fn names(paths: &[PathBuf]) -> String {
    let names: Vec<_> = paths
        .iter()
        .map(|p| {
            p.file_name()
                .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
        })
        .collect();
    names.join(", ")
}

fn saves() -> &'static Arc<Saves> {
    static SAVES: OnceLock<Arc<Saves>> = OnceLock::new();
    SAVES.get_or_init(Default::default)
}

fn queue(saves: &Arc<Saves>, path: PathBuf, whole: bool, job: Job) {
    let mut state = saves.lock();
    if let Some(waiting) = state.files.get_mut(&path) {
        // its thread runs and takes it
        if whole {
            waiting.clear();
        }
        waiting.push_back(job);
        return;
    }
    state.files.insert(path.clone(), VecDeque::from([job]));
    drop(state);
    let worker = Arc::clone(saves);
    let thread_path = path.clone();
    let spawned = std::thread::Builder::new()
        .name("aurora-save".into())
        .spawn(move || worker.run(&thread_path));
    if let Err(e) = spawned {
        // written here, as before the save threads
        log::warn!("saves: no thread, written on the caller: {e}");
        saves.run(&path);
    }
}

impl Saves {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Write the saves of `path` until none is left.
    fn run(&self, path: &Path) {
        loop {
            let job = {
                let mut state = self.lock();
                match state.files.get_mut(path).and_then(VecDeque::pop_front) {
                    Some(job) => job,
                    None => {
                        state.files.remove(path);
                        self.done.notify_all();
                        return;
                    }
                }
            };
            // (a panic is logged by the hook; the next saves still go)
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
        }
    }

    /// Until `path` (or every file) has no save left, `timeout` at most:
    /// the files still waiting.
    fn wait(&self, path: Option<&Path>, timeout: Duration) -> Vec<PathBuf> {
        let deadline = Instant::now() + timeout;
        let mut state = self.lock();
        loop {
            let left: Vec<PathBuf> = match path {
                Some(path) => state.files.get_key_value(path).map(|(p, _)| p.clone()).into_iter().collect(),
                None => state.files.keys().cloned().collect(),
            };
            let time = deadline.saturating_duration_since(Instant::now());
            if left.is_empty() || time.is_zero() {
                return left;
            }
            state = self
                .done
                .wait_timeout(state, time)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Saves, queue};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Barrier, Mutex};
    use std::time::Duration;

    const LONG: Duration = Duration::from_secs(10);

    /// A save that notes `n` in `written` once it runs.
    fn note(written: &Arc<Mutex<Vec<u32>>>, n: u32) -> Box<dyn FnOnce() + Send> {
        let written = Arc::clone(written);
        Box::new(move || written.lock().expect("written").push(n))
    }

    /// A save that holds its file's thread until the test lets it go.
    fn held(written: &Arc<Mutex<Vec<u32>>>, n: u32) -> (Box<dyn FnOnce() + Send>, Arc<Barrier>, Arc<Barrier>) {
        let (started, gate) = (Arc::new(Barrier::new(2)), Arc::new(Barrier::new(2)));
        let (w, s, g) = (Arc::clone(written), Arc::clone(&started), Arc::clone(&gate));
        let job = Box::new(move || {
            s.wait();
            g.wait();
            w.lock().expect("written").push(n);
        });
        (job, started, gate)
    }

    fn file(name: &str) -> PathBuf {
        Path::new("saves-test").join(name)
    }

    #[test]
    fn a_whole_file_save_not_started_is_replaced_by_the_newer_one() {
        let saves = Arc::new(Saves::default());
        let written = Arc::new(Mutex::new(Vec::new()));
        // the first save holds the thread while three more are asked
        let (first, started, gate) = held(&written, 0);
        queue(&saves, file("a"), true, first);
        started.wait();
        for n in 1..=3 {
            queue(&saves, file("a"), true, note(&written, n));
        }
        gate.wait();
        assert!(saves.wait(None, LONG).is_empty());
        assert_eq!(*written.lock().expect("written"), vec![0, 3]);
    }

    #[test]
    fn changes_of_a_file_all_run_in_their_order() {
        let saves = Arc::new(Saves::default());
        let written = Arc::new(Mutex::new(Vec::new()));
        let (first, started, gate) = held(&written, 0);
        queue(&saves, file("a"), false, first);
        started.wait();
        for n in 1..=3 {
            queue(&saves, file("a"), false, note(&written, n));
        }
        gate.wait();
        assert!(saves.wait(None, LONG).is_empty());
        assert_eq!(*written.lock().expect("written"), vec![0, 1, 2, 3]);
    }

    #[test]
    fn a_whole_file_save_drops_the_changes_before_it_and_keeps_those_after() {
        let saves = Arc::new(Saves::default());
        let written = Arc::new(Mutex::new(Vec::new()));
        let (first, started, gate) = held(&written, 0);
        queue(&saves, file("a"), false, first);
        started.wait();
        queue(&saves, file("a"), false, note(&written, 1));
        queue(&saves, file("a"), true, note(&written, 2));
        queue(&saves, file("a"), false, note(&written, 3));
        gate.wait();
        assert!(saves.wait(None, LONG).is_empty());
        assert_eq!(*written.lock().expect("written"), vec![0, 2, 3]);
    }

    #[test]
    fn a_file_held_does_not_hold_back_the_others() {
        let saves = Arc::new(Saves::default());
        let written = Arc::new(Mutex::new(Vec::new()));
        let (first, started, gate) = held(&written, 0);
        queue(&saves, file("slow"), true, first);
        started.wait();
        queue(&saves, file("quick"), true, note(&written, 1));
        assert!(saves.wait(Some(&file("quick")), LONG).is_empty());
        assert_eq!(*written.lock().expect("written"), vec![1]);
        // the held one is still left, and named
        assert_eq!(saves.wait(None, Duration::from_millis(20)), vec![file("slow")]);
        gate.wait();
        assert!(saves.wait(None, LONG).is_empty());
        assert_eq!(*written.lock().expect("written"), vec![1, 0]);
    }

    #[test]
    fn wait_returns_at_once_with_nothing_to_write_and_gives_up_after_its_timeout() {
        let saves = Arc::new(Saves::default());
        assert!(saves.wait(None, Duration::ZERO).is_empty());
        assert!(saves.wait(Some(&file("a")), Duration::ZERO).is_empty());
        let written = Arc::new(Mutex::new(Vec::new()));
        let (first, started, gate) = held(&written, 0);
        queue(&saves, file("a"), true, first);
        started.wait();
        assert_eq!(saves.wait(Some(&file("a")), Duration::from_millis(20)), vec![file("a")]);
        assert!(
            saves.wait(Some(&file("b")), Duration::ZERO).is_empty(),
            "another file has nothing to wait for"
        );
        gate.wait();
        assert!(saves.wait(Some(&file("a")), LONG).is_empty());
        assert_eq!(*written.lock().expect("written"), vec![0]);
    }

    #[test]
    fn a_save_that_panics_does_not_stop_the_next() {
        let saves = Arc::new(Saves::default());
        let written = Arc::new(Mutex::new(Vec::new()));
        let (first, started, gate) = held(&written, 0);
        queue(&saves, file("a"), false, first);
        started.wait();
        queue(&saves, file("a"), false, Box::new(|| panic!("a save that fails")));
        queue(&saves, file("a"), false, note(&written, 1));
        gate.wait();
        assert!(saves.wait(None, LONG).is_empty());
        assert_eq!(*written.lock().expect("written"), vec![0, 1]);
    }
}
