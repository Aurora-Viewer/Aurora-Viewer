//! The transcripts on disk: the folder of one account on one grid, the
//! thread that appends to the files and reads their end, and the reading
//! rules of LLLogChat::loadChatHistory (indra/newview/lllogchat.cpp,
//! originally LGPL 2.1).
//!
//! Nothing here runs on the main thread: it queues jobs, each with the full
//! path of its file, so a line queued for one account can never land in the
//! folder of the account logged in next. A file is opened, appended to with
//! one write and closed again for every batch of lines: no handle or lock is
//! kept, and two viewers logged in on the same account interleave whole
//! lines.

use super::format::{self, Entry};
use crossbeam_channel::{Receiver, Sender, TrySendError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use uuid::Uuid;

/// LOG_RECALL_SIZE: how much of the end of a transcript is read when a
/// conversation opens.
pub const LOG_RECALL_SIZE: u64 = 20480;
/// Longest line kept, in bytes (the fgets buffer of loadChatHistory); the
/// rest of a longer one is dropped.
pub const MAX_LINE: usize = 20480;
/// Longest message kept once its continuation lines are joined.
pub const MAX_MESSAGE: usize = 64 * 1024;
/// Lines waiting to be written; past this they are dropped (a disk that
/// stopped answering must not grow the memory without end).
const QUEUE: usize = 8192;

/// End of line of the files: Firestorm writes them through a text-mode
/// stream, so CRLF on Windows.
pub const EOL: &str = if cfg!(windows) { "\r\n" } else { "\n" };

/// Who the transcripts belong to. The same account id exists on the main
/// grid and on the beta grid, and other grids have their own ids: the grid
/// is part of the identity (Firestorm's folder is `<first_last>[.<grid>]`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Identity {
    pub grid: String,
    pub agent: Uuid,
}

/// Folder of the transcripts of an identity, under the folder that holds
/// one folder per account id: `<accounts>/<agent>/chat_logs/<grid>`.
pub fn folder(accounts: &Path, id: &Identity) -> PathBuf {
    accounts
        .join(id.agent.to_string())
        .join("chat_logs")
        .join(super::names::clean_file_name(&id.grid))
}

/// The end of a transcript read for a conversation.
#[derive(Debug)]
pub struct Loaded {
    /// The conversation (nil = nearby chat).
    pub conversation: Uuid,
    /// The login it was asked in.
    pub epoch: u64,
    pub entries: Vec<Entry>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SharedAppendTarget {
    pub epoch: u64,
    pub dir: PathBuf,
    pub peer: Option<Uuid>,
    pub stem: String,
    pub today: Option<(i64, i64, i64)>,
    pub scheme: super::names::NamingScheme,
}

#[derive(PartialEq, Eq)]
enum AppendTarget {
    Exact(PathBuf),
    Shared(SharedAppendTarget),
}

enum Job {
    Append {
        target: AppendTarget,
        text: String,
    },
    Tail {
        dir: PathBuf,
        stem: String,
        today: Option<(i64, i64, i64)>,
        conversation: Uuid,
        epoch: u64,
        shared: bool,
    },
    Erase {
        dir: PathBuf,
        done: Sender<std::io::Result<usize>>,
    },
    Move {
        from: PathBuf,
        to: PathBuf,
        done: Sender<std::io::Result<usize>>,
    },
    /// One file of an import (`import::merge_file`).
    Merge {
        source: PathBuf,
        target: PathBuf,
        done: Sender<std::io::Result<u64>>,
    },
    /// A whole small file read (the conversation log); None when it does
    /// not exist.
    Read {
        path: PathBuf,
        done: Sender<std::io::Result<Option<Vec<u8>>>>,
    },
    /// A whole file replaced at once (the conversation log).
    Publish {
        path: PathBuf,
        bytes: Vec<u8>,
        done: Sender<std::io::Result<()>>,
    },
    /// The conversation log's backups of a folder removed
    /// (LLConversationLog::deleteBackupLogs).
    RemoveBackups {
        dir: PathBuf,
    },
    /// Another job on the files of the account, in turn with the lines
    /// (the Chat Service's archives, its stitched reads and its delete,
    /// which must never interleave with a transcript write: the LL viewer's
    /// transcript mutation mutex, LLLogChat::saveHistory).
    Run(Box<dyn FnOnce() + Send>),
    /// Where an account's files go (`place::place`: it reads the folders).
    Place {
        request: super::place::Request,
        done: Sender<super::place::Placement>,
    },
    /// The checks of a folder picked in the preferences (`place::check`).
    Check {
        picked: PathBuf,
        agent: Uuid,
        account: Option<super::import::Account>,
        done: Sender<super::place::FolderCheck>,
    },
    /// The best existing file of a conversation, to open outside Aurora.
    Locate {
        dir: PathBuf,
        peer: Option<Uuid>,
        stem: String,
        today: Option<(i64, i64, i64)>,
        done: Sender<Option<PathBuf>>,
    },
    Sync(Sender<()>),
}

/// The way in for an import's thread: it waits for each file while the
/// lines of the conversations take their turn between two files.
pub struct MergePort(Sender<Job>);

impl MergePort {
    /// None when the transcript thread is gone.
    pub fn merge(&self, source: PathBuf, target: PathBuf) -> Option<std::io::Result<u64>> {
        let (done, rx) = crossbeam_channel::bounded(1);
        self.0.send(Job::Merge { source, target, done }).ok()?;
        rx.recv().ok()
    }
}

/// The transcript thread and its queue.
pub struct Worker {
    jobs: Sender<Job>,
    loaded: Receiver<Loaded>,
    /// The queue overflowed (said once).
    overflow: AtomicBool,
}

impl Worker {
    pub fn start() -> Option<Worker> {
        let (jobs, rx) = crossbeam_channel::bounded::<Job>(QUEUE);
        let (tx, loaded) = crossbeam_channel::unbounded();
        let spawned = std::thread::Builder::new().name("aurora-chatlog".into()).spawn(move || run(rx, tx));
        match spawned {
            Ok(_) => Some(Worker {
                jobs,
                loaded,
                overflow: AtomicBool::new(false),
            }),
            Err(e) => {
                log::warn!("conversation transcripts: no writer thread ({e})");
                None
            }
        }
    }

    /// Queue a job. False when the queue refused it (full, or the thread is
    /// gone): the job is dropped with its answer channel, which closes
    /// unanswered.
    fn queue(&self, job: Job) -> bool {
        match self.jobs.try_send(job) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                if !self.overflow.swap(true, Ordering::Relaxed) {
                    log::warn!("conversation transcripts: the disk does not keep up, some lines are not written");
                }
                false
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    }

    /// Queue a job without saying whether it was queued: a refused line is
    /// lost, and a refused job with an answer channel closes it unanswered.
    fn send(&self, job: Job) {
        self.queue(job);
    }

    /// Append text (whole lines, with their ends of line) to an exact path.
    pub fn append(&self, path: PathBuf, text: String) {
        self.send(Job::Append {
            target: AppendTarget::Exact(path),
            text,
        });
    }

    /// Resolve a shared conversation's append path on the `aurora-chatlog`
    /// worker thread (caching per session epoch) and append `text`.
    pub fn append_shared(&self, target: SharedAppendTarget, text: String) {
        self.send(Job::Append {
            target: AppendTarget::Shared(target),
            text,
        });
    }

    pub fn tail(&self, dir: PathBuf, stem: String, today: Option<(i64, i64, i64)>, conversation: Uuid, epoch: u64, shared: bool) {
        self.send(Job::Tail {
            dir,
            stem,
            today,
            conversation,
            epoch,
            shared,
        });
    }

    /// Erase the transcripts of a folder; the answer is the number of files
    /// removed.
    pub fn erase(&self, dir: PathBuf) -> Receiver<std::io::Result<usize>> {
        let (done, rx) = crossbeam_channel::bounded(1);
        self.send(Job::Erase { dir, done });
        rx
    }

    /// Move the transcripts of a folder to another, after the lines already
    /// queued; the answer is the number of files moved. None when the queue
    /// refused the move (full, or the thread is gone): nothing moves.
    pub fn relocate(&self, from: PathBuf, to: PathBuf) -> Option<Receiver<std::io::Result<usize>>> {
        let (done, rx) = crossbeam_channel::bounded(1);
        self.queue(Job::Move { from, to, done }).then_some(rx)
    }

    /// Read a whole small file, after what is queued; the answer is None
    /// when it does not exist.
    pub fn read(&self, path: PathBuf) -> Receiver<std::io::Result<Option<Vec<u8>>>> {
        let (done, rx) = crossbeam_channel::bounded(1);
        self.read_into(path, done);
        rx
    }

    /// `read`, answered through a channel made before the path was known
    /// (the conversation log started at login, read once placed).
    pub fn read_into(&self, path: PathBuf, done: Sender<std::io::Result<Option<Vec<u8>>>>) {
        self.send(Job::Read { path, done });
    }

    /// Where an account's files go, after what is queued: the answer of
    /// `place::place`, which reads the folders off the frame. None when the
    /// queue refused the job (full, or the thread is gone): no answer comes.
    pub fn place(&self, request: super::place::Request) -> Option<Receiver<super::place::Placement>> {
        let (done, rx) = crossbeam_channel::bounded(1);
        self.queue(Job::Place { request, done }).then_some(rx)
    }

    /// The checks of a folder picked for an account (`place::check`).
    pub fn check(&self, picked: PathBuf, agent: Uuid, account: Option<super::import::Account>) -> Receiver<super::place::FolderCheck> {
        let (done, rx) = crossbeam_channel::bounded(1);
        self.send(Job::Check {
            picked,
            agent,
            account,
            done,
        });
        rx
    }

    /// The best existing file of a conversation (`names::existing_source_file`).
    pub fn locate(&self, dir: PathBuf, peer: Option<Uuid>, stem: String, today: Option<(i64, i64, i64)>) -> Receiver<Option<PathBuf>> {
        let (done, rx) = crossbeam_channel::bounded(1);
        self.send(Job::Locate {
            dir,
            peer,
            stem,
            today,
            done,
        });
        rx
    }

    /// Replace a whole file (written beside it, then renamed), in the order
    /// of the calls. The answer says whether it is on disk; a job the queue
    /// refused closes the channel without one.
    pub fn publish(&self, path: PathBuf, bytes: Vec<u8>) -> Receiver<std::io::Result<()>> {
        let (done, rx) = crossbeam_channel::bounded(1);
        self.send(Job::Publish { path, bytes, done });
        rx
    }

    /// Remove the backups of the conversation log of a folder.
    pub fn remove_backups(&self, dir: PathBuf) {
        self.send(Job::RemoveBackups { dir });
    }

    /// Run `job` on the transcript thread after what is queued. False when
    /// the queue refused it (full, or the thread is gone): the job did not
    /// and will not run, and the caller answers for it.
    pub fn run(&self, job: impl FnOnce() + Send + 'static) -> bool {
        self.jobs.try_send(Job::Run(Box::new(job))).is_ok()
    }

    pub fn merge_port(&self) -> MergePort {
        MergePort(self.jobs.clone())
    }

    pub fn loaded(&self) -> impl Iterator<Item = Loaded> + '_ {
        self.loaded.try_iter()
    }

    /// Wait until everything queued so far is on disk (viewer exit, tests).
    pub fn flush(&self, timeout: Duration) -> bool {
        let (tx, rx) = crossbeam_channel::bounded(1);
        if self.jobs.send_timeout(Job::Sync(tx), timeout).is_err() {
            return false;
        }
        rx.recv_timeout(timeout).is_ok()
    }
}

/// The whole files whose publication was refused, the latest per file,
/// kept by the thread until one goes through: the conversation log of a
/// session that ended (a logout, the exit) no longer reads the answer of its
/// last publication, so the thread tries it again itself, waiting longer
/// each time (`save_retry`), before an exit's `flush` and when the queue
/// closes. Until then a read of that file answers with these bytes, so the
/// next session of the account starts from them.
#[derive(Default)]
struct Owed {
    files: Vec<(PathBuf, Vec<u8>)>,
    backoff: crate::save_retry::Backoff,
}

impl Owed {
    /// The answer of a publication: a refused one is kept, a written one
    /// replaces what was owed for that file.
    fn published(&mut self, path: &Path, bytes: Vec<u8>, written: &std::io::Result<()>) {
        self.files.retain(|(p, _)| p != path);
        match written {
            Ok(()) => {
                if self.files.is_empty() {
                    self.backoff.succeeded();
                }
            }
            Err(_) => {
                self.files.push((path.to_owned(), bytes));
                if self.backoff.due(std::time::Instant::now()) {
                    self.backoff.failed(std::time::Instant::now());
                }
            }
        }
    }

    fn read(&self, path: &Path) -> Option<Vec<u8>> {
        self.files.iter().find(|(p, _)| p == path).map(|(_, b)| b.clone())
    }

    /// Every owed file tried again; the next try waits longer if one is
    /// still refused.
    fn retry(&mut self) {
        if self.files.is_empty() {
            return;
        }
        let mut refused = None;
        self.files.retain(|(path, bytes)| match publish(path, bytes) {
            Ok(()) => false,
            Err(e) => {
                refused.get_or_insert(e);
                true
            }
        });
        match refused {
            None => {
                log::info!("conversation log: saved on a later try");
                self.backoff.succeeded();
            }
            Some(e) => {
                let wait = self.backoff.failed(std::time::Instant::now());
                log::warn!("conversation log: still not saved ({e}); tried again in {} s", wait.as_secs());
            }
        }
    }

    /// The next job; while files are owed, they are tried again when the
    /// wait is over, and once more when the queue closes.
    fn next(&mut self, jobs: &Receiver<Job>) -> Option<Job> {
        loop {
            if self.files.is_empty() {
                return jobs.recv().ok();
            }
            match jobs.recv_timeout(self.backoff.left(std::time::Instant::now())) {
                Ok(job) => return Some(job),
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => self.retry(),
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    self.retry();
                    return None;
                }
            }
        }
    }
}

fn publish(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    path.parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| crate::atomic_file::write(path, bytes))
}

type SharedKey = (PathBuf, Option<Uuid>, String, Option<(i64, i64, i64)>, super::names::NamingScheme);

/// The files the shared targets of one append epoch go to, so a line does
/// not list the folder again. A file is resolved again once the local date
/// changed since (a dated file the Legacy Viewer starts for the new month)
/// or when it is missing (renamed or moved by the other viewer), so a file
/// the Legacy Viewer starts during the session is followed.
#[derive(Default)]
struct SharedTargets {
    epoch: u64,
    files: std::collections::HashMap<SharedKey, (PathBuf, (i64, i64, i64))>,
}

impl SharedTargets {
    /// The file of `shared` on `date`: the one resolved before while it is
    /// still valid, else `resolve()`'s.
    fn target(
        &mut self,
        shared: &SharedAppendTarget,
        date: (i64, i64, i64),
        exists: impl Fn(&Path) -> bool,
        resolve: impl FnOnce() -> Option<PathBuf>,
    ) -> Option<PathBuf> {
        if self.epoch != shared.epoch {
            self.files.clear();
            self.epoch = shared.epoch;
        }
        let key = (
            shared.dir.clone(),
            shared.peer,
            shared.stem.to_lowercase(),
            shared.today,
            shared.scheme,
        );
        if let Some((path, on)) = self.files.get(&key)
            && *on == date
            && exists(path)
        {
            return Some(path.clone());
        }
        let path = resolve()?;
        self.files.insert(key, (path.clone(), date));
        Some(path)
    }
}

fn resolve_append_target(target: &AppendTarget, shared_targets: &mut SharedTargets) -> Option<PathBuf> {
    match target {
        AppendTarget::Exact(path) => Some(path.clone()),
        AppendTarget::Shared(shared) => shared_targets.target(
            shared,
            super::names::local_date(),
            |p| p.is_file(),
            || super::names::shared_file_path(&shared.dir, shared.peer, &shared.stem, shared.today, shared.scheme),
        ),
    }
}

/// The queue's receiving end, emptied when it is dropped: the thread ended,
/// by a panic too. The jobs left in it go with their answer channels, which
/// close unanswered; a bounded queue would keep them while a sender lives,
/// and a placement or a move would wait for ever.
struct Drained(Receiver<Job>);

impl std::ops::Deref for Drained {
    type Target = Receiver<Job>;

    fn deref(&self) -> &Receiver<Job> {
        &self.0
    }
}

impl Drop for Drained {
    fn drop(&mut self) {
        while self.0.try_recv().is_ok() {}
    }
}

/// The queue of a worker that has no thread (tests): its jobs wait there,
/// never run, and are dropped unanswered with it, as when the thread dies
/// with jobs queued.
#[cfg(test)]
pub struct Stalled {
    _queue: Drained,
}

#[cfg(test)]
impl Worker {
    /// A worker without a thread, and its queue (tests). Dropping the queue
    /// first makes a worker whose thread is gone: every job is refused.
    pub fn stalled() -> (Worker, Stalled) {
        let (jobs, rx) = crossbeam_channel::bounded::<Job>(QUEUE);
        let (_, loaded) = crossbeam_channel::unbounded();
        let worker = Worker {
            jobs,
            loaded,
            overflow: AtomicBool::new(false),
        };
        (worker, Stalled { _queue: Drained(rx) })
    }
}

fn run(jobs: Receiver<Job>, loaded: Sender<Loaded>) {
    let jobs = Drained(jobs);
    // a failure is said once per run, without the file name (it is the name
    // of a resident or a group, and the viewer's log may be shared)
    let mut failed = false;
    let mut owed = Owed::default();
    let mut shared_targets = SharedTargets::default();
    let mut next = owed.next(&jobs);
    while let Some(job) = next.take() {
        match job {
            Job::Append { target, mut text } => {
                // the lines already waiting for the same target go with it
                loop {
                    match jobs.try_recv() {
                        Ok(Job::Append { target: t2, text: t }) if t2 == target => text.push_str(&t),
                        Ok(other) => {
                            next = Some(other);
                            break;
                        }
                        Err(_) => break,
                    }
                }
                if let Some(path) = resolve_append_target(&target, &mut shared_targets)
                    && let Err(e) = append(&path, text.as_bytes())
                    && !failed
                {
                    failed = true;
                    log::warn!(
                        "conversation transcripts: cannot write in {} ({e}); the conversations go on without it",
                        path.parent().unwrap_or(Path::new("")).display()
                    );
                }
            }
            Job::Tail {
                dir,
                stem,
                today,
                conversation,
                epoch,
                shared,
            } => {
                let entries = if shared {
                    // every spelling and dated file, as the History window
                    // lists them (the newest few read)
                    let peer = (!conversation.is_nil()).then_some(conversation);
                    let paths = super::names::all_candidate_files(&dir, peer, &stem);
                    read_combined_tail(&paths)
                } else {
                    super::names::existing_file(&dir, &stem, today)
                        .and_then(|path| read_tail(&path).ok())
                        .unwrap_or_default()
                };
                let _ = loaded.send(Loaded {
                    conversation,
                    epoch,
                    entries,
                });
            }
            Job::Erase { dir, done } => {
                shared_targets.files.clear();
                let _ = done.send(erase(&dir));
            }
            Job::Move { from, to, done } => {
                // the session publishes its log again in the new folder: a
                // copy owed in the old one would bring the folder back
                owed.files.retain(|(p, _)| !p.starts_with(&from));
                shared_targets.files.clear();
                let _ = done.send(relocate(&from, &to));
            }
            Job::Merge { source, target, done } => {
                shared_targets.files.clear();
                let _ = done.send(super::import::merge_file(&source, &target));
            }
            Job::Read { path, done } => {
                let read = match owed.read(&path) {
                    Some(bytes) => Ok(Some(bytes)),
                    None => match std::fs::read(&path) {
                        Ok(bytes) => {
                            crate::disk_io::read(crate::disk_io::Origin::Logs, bytes.len());
                            Ok(Some(bytes))
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                        Err(e) => Err(e),
                    },
                };
                let _ = done.send(read);
            }
            Job::Publish { path, bytes, done } => {
                let written = publish(&path, &bytes);
                if let Err(e) = &written {
                    log::warn!("conversation log: not saved ({e}); it will be tried again");
                }
                owed.published(&path, bytes, &written);
                let _ = done.send(written);
            }
            Job::Place { request, done } => {
                let _ = done.send(super::place::place(&request));
            }
            Job::Check {
                picked,
                agent,
                account,
                done,
            } => {
                let _ = done.send(super::place::check(&picked, agent, account.as_ref()));
            }
            Job::Locate {
                dir,
                peer,
                stem,
                today,
                done,
            } => {
                let _ = done.send(super::names::existing_source_file(&dir, peer, &stem, today));
            }
            Job::RemoveBackups { dir } => {
                for backup in log_backups(&dir) {
                    if let Err(e) = std::fs::remove_file(&backup) {
                        log::warn!("conversation log: a backup is not removed ({e})");
                    }
                }
            }
            Job::Run(job) => job(),
            Job::Sync(done) => {
                // the exit's flush: what is owed is tried once more first
                owed.retry();
                let _ = done.send(());
            }
        }
        if next.is_none() {
            next = owed.next(&jobs);
        }
    }
}

fn append(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let open = || std::fs::OpenOptions::new().append(true).create(true).open(path);
    let mut file = match open() {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            open()?
        }
        Err(e) => return Err(e),
    };
    // one write: an append-mode write lands whole at the end of the file,
    // also when another viewer appends to it
    file.write_all(bytes)?;
    crate::disk_io::written(crate::disk_io::Origin::Logs, bytes.len());
    Ok(())
}

/// Remove the transcript files of a folder (and the folder once empty).
/// Only `.txt` files directly in it: nothing else is ever touched. Only
/// `ChatLog` decides which folder: never a shared one (`ChatLog::shared`).
pub(super) fn erase(dir: &Path) -> std::io::Result<usize> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut removed = 0;
    let mut first_error = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_transcript = path.extension().is_some_and(|x| x.eq_ignore_ascii_case(super::names::EXTENSION))
            && entry.file_type().is_ok_and(|t| t.is_file());
        if !is_transcript {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(e) => first_error = first_error.or(Some(e)),
        }
    }
    let _ = std::fs::remove_dir(dir);
    match first_error {
        Some(e) => Err(e),
        None => Ok(removed),
    }
}

/// The transcripts of `from` moved into `to` (LLFloaterPreference::
/// moveTranscriptsAndLog): every `.txt` file directly in it, renamed, or
/// copied then removed when the folders are on different disks. A file
/// already in `to`, or any failure, puts back what was moved and fails:
/// the transcripts are never split between two folders.
fn relocate(from: &Path, to: &Path) -> std::io::Result<usize> {
    let files: Vec<PathBuf> = match std::fs::read_dir(from) {
        Ok(entries) => entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(super::names::EXTENSION)))
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    std::fs::create_dir_all(to)?;
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    for source in files {
        let Some(name) = source.file_name() else {
            continue;
        };
        let target = to.join(name);
        let result = if target.exists() {
            Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "a transcript of the same name is there",
            ))
        } else {
            move_file(&source, &target)
        };
        if let Err(e) = result {
            for (source, target) in moved.iter().rev() {
                let _ = move_file(target, source);
            }
            return Err(e);
        }
        moved.push((source, target));
    }
    // LLConversationLog::moveLog: the log follows, the one already there
    // kept as a backup
    // a log that cannot follow fails the whole move: the transcripts go
    // back, so the log and its transcripts are never split between folders
    if let Err(e) = move_log(&from.join(super::conversations::FILE), &to.join(super::conversations::FILE)) {
        log::warn!("conversation log: not moved with the transcripts ({e}); the move is undone");
        for (source, target) in moved.iter().rev() {
            let _ = move_file(target, source);
        }
        return Err(e);
    }
    let _ = std::fs::remove_dir(from);
    Ok(moved.len())
}

/// LLConversationLog::moveLog: `from` becomes `to`; a file already at `to`
/// is renamed `<to>.backup`, `.backup1`, `.backup2`… first.
fn move_log(from: &Path, to: &Path) -> std::io::Result<()> {
    if !from.is_file() {
        return Ok(());
    }
    if to.is_file() {
        let name = to.file_name().and_then(|n| n.to_str()).unwrap_or(super::conversations::FILE);
        let mut backup = to.with_file_name(format!("{name}.backup"));
        let mut n = 0;
        while backup.exists() {
            n += 1;
            backup = to.with_file_name(format!("{name}.backup{n}"));
        }
        std::fs::rename(to, &backup)?;
        if let Err(e) = move_file(from, to) {
            // the log already there is put back
            let _ = std::fs::rename(&backup, to);
            return Err(e);
        }
        return Ok(());
    }
    move_file(from, to)
}

/// The backups of the conversation log in a folder
/// (LLConversationLog::getListOfBackupLogs: `conversation.log.backup*`).
fn log_backups(dir: &Path) -> Vec<PathBuf> {
    let prefix = format!("{}.backup", super::conversations::FILE);
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.starts_with(&prefix)))
        .map(|e| e.path())
        .collect()
}

fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    if let Err(e) = std::fs::remove_file(from) {
        let _ = std::fs::remove_file(to);
        return Err(e);
    }
    Ok(())
}

/// The messages at the end of a transcript, like loadChatHistory: the last
/// `LOG_RECALL_SIZE` bytes, without the line the cut fell in.
pub fn read_tail(path: &Path) -> std::io::Result<Vec<Entry>> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let cut = len > LOG_RECALL_SIZE - 1;
    if cut {
        file.seek(SeekFrom::Start(len - (LOG_RECALL_SIZE - 1)))?;
    }
    let mut bytes = Vec::with_capacity(LOG_RECALL_SIZE as usize);
    file.take(LOG_RECALL_SIZE).read_to_end(&mut bytes)?;
    crate::disk_io::read(crate::disk_io::Origin::Logs, bytes.len());
    Ok(parse(&bytes, cut))
}

/// Unix second timestamp (when known) and duplicate-detection key for an
/// entry: `{minute}\0{from}\0{text}` for dated entries, or
/// `{stamp}\0{from}\0{text}` when undated (the importer's rule).
pub(super) fn entry_stamp(entry: &Entry) -> (Option<i64>, String) {
    let sec = entry_time(entry);
    let key = match sec.map(|s| s / 60) {
        Some(minute) => format!("{minute}\0{}\0{}", entry.from, entry.text),
        None => format!("{}\0{}\0{}", entry.stamp, entry.from, entry.text),
    };
    (sec, key)
}

/// The moment of an entry whose stamp has a date, in Unix seconds.
pub(super) fn entry_time(entry: &Entry) -> Option<i64> {
    entry
        .time
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
}

/// The key duplicates are found by when the files of one conversation are
/// combined: minute, sender and text, for a line whose stamp has a date. A
/// line with a time only (LogTimestampDate off) has none: the same short
/// line at the same minute on two days must not be taken for one.
pub(super) fn dedup_key(entry: &Entry) -> Option<String> {
    entry_time(entry).map(|sec| format!("{}\0{}\0{}", sec / 60, entry.from, entry.text))
}

/// The times the lines of one file are ordered by in a combined history: a
/// dated line its own; one without a date that of the dated line before it
/// in its file, or of the file's first dated line when none is before it;
/// and every line of a file without any date the file's modification time,
/// so that file falls in place by when it was written, not by its place
/// among the files.
pub(super) fn line_times(dated: &[Option<i64>], modified: i64) -> Vec<i64> {
    let mut last = dated.iter().flatten().next().copied().unwrap_or(modified);
    dated
        .iter()
        .map(|t| {
            if let Some(t) = t {
                last = *t;
            }
            last
        })
        .collect()
}

/// A file's modification time in Unix seconds (0 when unknown).
pub(super) fn modified_secs(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}

struct StampedEntry {
    entry: Entry,
    key: Option<String>,
    at: i64,
}

fn stamp_entries(entries: Vec<Entry>, modified: i64) -> Vec<StampedEntry> {
    let times: Vec<Option<i64>> = entries.iter().map(entry_time).collect();
    entries
        .into_iter()
        .zip(line_times(&times, modified))
        .map(|(entry, at)| StampedEntry {
            key: dedup_key(&entry),
            entry,
            at,
        })
        .collect()
}

/// `theirs` (the next file) merged into `ours` by time, without its dated
/// lines `ours` already has (as many times as it has them: a line repeated
/// within one file stays); lines without a date are all kept.
fn combine_entries(ours: Vec<StampedEntry>, theirs: Vec<StampedEntry>) -> Vec<StampedEntry> {
    if ours.is_empty() {
        return theirs;
    }
    let mut have: std::collections::HashMap<&str, u32> = std::collections::HashMap::with_capacity(ours.len());
    for m in &ours {
        if let Some(key) = &m.key {
            *have.entry(key.as_str()).or_default() += 1;
        }
    }
    let new: Vec<StampedEntry> = theirs
        .into_iter()
        .filter(|m| match m.key.as_deref().and_then(|key| have.get_mut(key)) {
            Some(count) if *count > 0 => {
                *count -= 1;
                false
            }
            _ => true,
        })
        .collect();
    if new.is_empty() {
        return ours;
    }
    let mut out = Vec::with_capacity(ours.len() + new.len());
    let mut rest = ours.into_iter().peekable();
    for m in new {
        while let Some(own) = rest.next_if(|own| own.at <= m.at) {
            out.push(own);
        }
        out.push(m);
    }
    out.extend(rest);
    out
}

/// How many of a conversation's files the history shown when it opens
/// reads, the most recently written first: that history is the end of the
/// conversation, which the files written last hold.
const TAIL_FILES: usize = 4;

fn read_whole(path: &Path) -> std::io::Result<Vec<Entry>> {
    let bytes = std::fs::read(path)?;
    crate::disk_io::read(crate::disk_io::Origin::Logs, bytes.len());
    Ok(parse(&bytes, false))
}

fn read_combined(paths: &[PathBuf], limit: Option<usize>, read: fn(&Path) -> std::io::Result<Vec<Entry>>) -> Vec<Entry> {
    let mut files: Vec<(i64, &PathBuf)> = paths.iter().map(|p| (modified_secs(p), p)).collect();
    // the oldest first (the candidates' order on equal times): on equal
    // line times its lines come first
    files.sort_by_key(|(modified, _)| *modified);
    let skip = limit.map_or(0, |n| files.len().saturating_sub(n));
    let mut combined = Vec::new();
    for (modified, path) in files.into_iter().skip(skip) {
        if let Ok(entries) = read(path) {
            combined = combine_entries(combined, stamp_entries(entries, modified));
        }
    }
    combined.into_iter().map(|m| m.entry).collect()
}

/// Read the tail of a conversation's files and combine them in time order,
/// without duplicate dated lines (minute + sender + text), the newest
/// `TAIL_FILES` only, by modification time.
pub fn read_combined_tail(paths: &[PathBuf]) -> Vec<Entry> {
    read_combined(paths, Some(TAIL_FILES), read_tail)
}

/// Read all entries across `paths` in time order, deduplicating dated lines
/// across files while keeping undated entries and within-file multiplicity.
pub(crate) fn read_combined_files(paths: &[PathBuf]) -> Vec<Entry> {
    read_combined(paths, None, read_whole)
}

/// Messages of a piece of transcript. `partial_start`: the piece starts in
/// the middle of the file, its first line is dropped.
pub fn parse(bytes: &[u8], partial_start: bool) -> Vec<Entry> {
    let mut reader = Messages::default();
    let mut out = Vec::new();
    let mut lines = bytes.split(|&b| b == b'\n');
    if partial_start {
        lines.next();
    }
    for line in lines {
        if let Some(done) = reader.line(line) {
            out.push(done);
        }
    }
    out.extend(reader.finish());
    out
}

/// Joins the lines of a transcript into messages, one line at a time.
#[derive(Default)]
pub struct Messages {
    current: Option<Entry>,
}

impl Messages {
    pub fn is_open(&self) -> bool {
        self.current.is_some()
    }

    /// Give the next line (without its '\n'); returns the message it ends,
    /// if any. Tolerates CRLF, a byte order mark and invalid UTF-8, and cuts
    /// a line longer than `MAX_LINE` at a character.
    pub fn line(&mut self, raw: &[u8]) -> Option<Entry> {
        let mut raw = raw;
        while let [rest @ .., b'\r' | b'\n'] = raw {
            raw = rest;
        }
        // remove_utf8_bom, on every line as Firestorm does
        let raw = raw.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(raw);
        let mut end = raw.len().min(MAX_LINE);
        // not in the middle of a character
        while end < raw.len() && end > 0 && raw[end] & 0xC0 == 0x80 {
            end -= 1;
        }
        let text = String::from_utf8_lossy(&raw[..end]);
        let line = cut_at_char(&text, MAX_LINE);
        // the lines after the first of a message start with a space; an
        // empty line is a paragraph break of the old format
        let more = if line.is_empty() { Some("") } else { format::continuation(line) };
        if let Some(more) = more {
            if let Some(cur) = &mut self.current
                && cur.text.len() + more.len() < MAX_MESSAGE
            {
                cur.text.push('\n');
                cur.text.push_str(more);
            }
            return None;
        }
        let done = self.finish();
        self.current = format::parse_line(line);
        done
    }

    /// The last message.
    pub fn finish(&mut self) -> Option<Entry> {
        let mut done = self.current.take()?;
        // LLLogChat mention form back to the link
        done.text = format::decode_mentions(&done.text);
        let kept = done.text.trim_end_matches('\n').len();
        done.text.truncate(kept);
        Some(done)
    }
}

fn cut_at_char(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    &s[..cut]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aurora-chatlog-store-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A move the queue refuses says so at once; one it took and the
    /// thread never runs closes its answer instead of leaving it waiting.
    #[test]
    fn a_refused_or_dropped_move_never_leaves_its_answer_waiting() {
        let (a, b) = (PathBuf::from("a"), PathBuf::from("b"));
        // the thread gone
        let (w, queue) = Worker::stalled();
        drop(queue);
        assert!(w.relocate(a.clone(), b.clone()).is_none());
        // the queue full
        let (w, _queue) = Worker::stalled();
        for _ in 0..QUEUE {
            w.append(PathBuf::from("Jane Doe.txt"), String::new());
        }
        assert!(w.relocate(a.clone(), b.clone()).is_none());
        // taken, then the thread gone with it queued
        let (w, queue) = Worker::stalled();
        let answer = w.relocate(a.clone(), b.clone()).expect("queued");
        assert!(matches!(answer.try_recv(), Err(crossbeam_channel::TryRecvError::Empty)));
        drop(queue);
        assert!(matches!(answer.try_recv(), Err(crossbeam_channel::TryRecvError::Disconnected)));
        // a thread that dies on a job: those queued behind it are dropped
        // with it, their answers closed
        let w = Worker::start().unwrap();
        let (go, wait) = crossbeam_channel::bounded::<()>(0);
        assert!(w.run(move || {
            let _ = wait.recv();
            panic!("a transcript thread that dies (test)");
        }));
        let answer = w.relocate(a, b).expect("queued");
        go.send(()).unwrap();
        assert!(matches!(
            answer.recv_timeout(Duration::from_secs(10)),
            Err(crossbeam_channel::RecvTimeoutError::Disconnected)
        ));
    }

    /// The session that published is gone (a logout reads no answer): the
    /// refused log is read back from memory by the next session and written
    /// by the exit's flush once the folder takes writes.
    #[test]
    fn a_refused_publication_is_kept_read_back_and_written_at_the_flush() {
        let root = temp();
        let dir = root.join("account");
        std::fs::write(&dir, b"in the way").unwrap();
        let path = dir.join("conversation.log");
        let worker = Worker::start().expect("thread");
        let answer = worker.publish(path.clone(), b"session log".to_vec());
        assert!(answer.recv_timeout(Duration::from_secs(10)).expect("answer").is_err());
        let read = worker.read(path.clone()).recv_timeout(Duration::from_secs(10)).expect("answer");
        assert_eq!(read.unwrap().as_deref(), Some(&b"session log"[..]), "read from memory");
        std::fs::remove_file(&dir).unwrap();
        assert!(worker.flush(Duration::from_secs(10)));
        assert_eq!(std::fs::read(&path).unwrap(), b"session log");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn folders_are_per_grid_and_account() {
        let root = Path::new("accounts");
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let id = |grid: &str, agent| Identity { grid: grid.into(), agent };
        assert_eq!(
            folder(root, &id("agni", a)),
            root.join(a.to_string()).join("chat_logs").join("agni")
        );
        assert_ne!(folder(root, &id("agni", a)), folder(root, &id("aditi", a)));
        assert_ne!(folder(root, &id("agni", a)), folder(root, &id("agni", b)));
        // a grid name cannot leave the account's folder
        let odd = folder(root, &id("../../x", a));
        assert_eq!(odd.parent(), folder(root, &id("agni", a)).parent());
    }

    #[test]
    fn reads_only_the_end_of_a_long_transcript() {
        let dir = temp();
        let path = dir.join("Jane Doe.txt");
        let mut text = String::new();
        for i in 0..5000 {
            text.push_str(&format!("[2026/10/12 14:05]  Jane Doe: message number {i}{EOL}"));
        }
        std::fs::write(&path, &text).unwrap();
        let entries = read_tail(&path).unwrap();
        assert!(entries.len() > 100 && entries.len() < 1000, "{}", entries.len());
        // whole messages only, the last ones, in order
        assert_eq!(entries.last().unwrap().text, "message number 4999");
        let first: usize = entries[0].text.rsplit(' ').next().unwrap().parse().unwrap();
        for (i, e) in entries.iter().enumerate() {
            assert_eq!(e.from, "Jane Doe");
            assert_eq!(e.text, format!("message number {}", first + i));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_short_transcript_is_read_whole() {
        let dir = temp();
        let path = dir.join("chat.txt");
        std::fs::write(
            &path,
            b"\xEF\xBB\xBF[2026/10/12 14:05]  Jane Doe: first\r\n[2026/10/12 14:06]  Second Life: second\n",
        )
        .unwrap();
        let entries = read_tail(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!((entries[0].from.as_str(), entries[0].text.as_str()), ("Jane Doe", "first"));
        assert_eq!(entries[0].stamp, "2026/10/12 14:05");
        assert!(entries[1].is_system());
        assert!(read_tail(&dir.join("missing.txt")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn joins_multi_line_messages_and_survives_bad_bytes() {
        let mut bytes = b"[2026/10/12 14:05]  Jane Doe: one\r\n two\r\n \r\n four\r\n\
            [2026/10/12 14:06]  Caf\xE9 \xFF: bad \xC3\x28 bytes\n\
            \n"
        .to_vec();
        bytes.extend("[2026/10/12 14:07]  Ælfrëd 猫 (alfred.cat): fin".as_bytes());
        let entries = parse(&bytes, false);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].text, "one\ntwo\n\nfour");
        assert_eq!(entries[1].from, "Caf\u{FFFD} \u{FFFD}");
        assert_eq!(entries[1].text, "bad \u{FFFD}( bytes");
        // a last line without an end of line is still a message
        assert_eq!(
            (entries[2].from.as_str(), entries[2].text.as_str()),
            ("Ælfrëd 猫 (alfred.cat)", "fin")
        );
        // a continuation with nothing before it is dropped
        assert!(parse(b" orphan\n", false).is_empty());
        // the first line of a piece cut out of a file is dropped
        let cut = parse(b"alf of a line\n[2026/10/12 14:07]  Jane Doe: whole\n", true);
        assert_eq!(cut.len(), 1);
        assert_eq!(cut[0].text, "whole");
    }

    #[test]
    fn one_line_is_bounded_and_cut_at_a_character() {
        let mut bytes = b"[2026/10/12 14:05]  Jane Doe: ".to_vec();
        let prefix = bytes.len();
        // 3-byte characters: the bound falls inside one
        bytes.extend("猫".repeat(20_000).as_bytes());
        bytes.extend(b"\n[2026/10/12 14:06]  Jane Doe: next\n");
        let entries = parse(&bytes, false);
        assert_eq!(entries.len(), 2);
        assert!(entries[0].text.len() <= MAX_LINE - prefix);
        assert!(entries[0].text.len() > MAX_LINE - prefix - 4);
        assert!(entries[0].text.chars().all(|c| c == '猫'));
        assert_eq!(entries[1].text, "next");
        // a message of many lines is bounded too
        let mut many = b"[2026/10/12 14:05]  Jane Doe: start\n".to_vec();
        for _ in 0..20_000 {
            many.extend(b" more more more\n");
        }
        let entries = parse(&many, false);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].text.len() <= MAX_MESSAGE);
    }

    #[test]
    fn appends_whole_lines_and_creates_the_folder() {
        let dir = temp().join("deep").join("er");
        let path = dir.join("Jane Doe.txt");
        let w = Worker::start().unwrap();
        for i in 0..50 {
            w.append(path.clone(), format!("[2026/10/12 14:05]  Jane Doe: line {i}{EOL}"));
        }
        assert!(w.flush(Duration::from_secs(10)));
        let bytes = std::fs::read(&path).unwrap();
        // UTF-8 without a byte order mark
        assert!(bytes.starts_with(b"[2026"));
        let entries = parse(&bytes, false);
        assert_eq!(entries.len(), 50);
        assert_eq!(entries[49].text, "line 49");
        std::fs::remove_dir_all(dir.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn a_folder_that_cannot_be_written_loses_nothing_else() {
        let dir = temp();
        // a file where the folder should be
        let blocked = dir.join("not-a-folder");
        std::fs::write(&blocked, b"x").unwrap();
        let w = Worker::start().unwrap();
        w.append(blocked.join("Jane Doe.txt"), "a\n".into());
        w.append(blocked.join("Jane Doe.txt"), "b\n".into());
        w.append(dir.join("ok.txt"), "c\n".into());
        assert!(w.flush(Duration::from_secs(10)));
        assert_eq!(std::fs::read(dir.join("ok.txt")).unwrap(), b"c\n");
        assert_eq!(std::fs::read(&blocked).unwrap(), b"x");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn erasing_removes_the_transcripts_only() {
        let dir = temp();
        let logs = dir.join("logs");
        std::fs::create_dir_all(logs.join("sub")).unwrap();
        std::fs::write(logs.join("chat.txt"), b"a").unwrap();
        std::fs::write(logs.join("Jane Doe.TXT"), b"b").unwrap();
        std::fs::write(logs.join("keep.json"), b"c").unwrap();
        std::fs::write(logs.join("sub").join("deep.txt"), b"d").unwrap();
        let w = Worker::start().unwrap();
        assert_eq!(w.erase(logs.clone()).recv_timeout(Duration::from_secs(10)).unwrap().unwrap(), 2);
        assert!(!logs.join("chat.txt").exists());
        assert!(logs.join("keep.json").exists());
        assert!(logs.join("sub").join("deep.txt").exists());
        // nothing there: nothing to do
        assert_eq!(w.erase(dir.join("none")).recv_timeout(Duration::from_secs(10)).unwrap().unwrap(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn moving_takes_the_queued_lines_and_never_splits_the_transcripts() {
        let dir = temp();
        let (old, new) = (dir.join("old"), dir.join("new"));
        let w = Worker::start().unwrap();
        w.append(old.join("Jane Doe.txt"), "a\n".into());
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("keep.json"), b"c").unwrap();
        // the line queued before the move goes with it
        let moved = w
            .relocate(old.clone(), new.clone())
            .expect("queued")
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert_eq!(moved.unwrap(), 1);
        assert_eq!(std::fs::read(new.join("Jane Doe.txt")).unwrap(), b"a\n");
        assert!(old.join("keep.json").exists(), "only transcripts move");
        // a transcript of the same name in the target: nothing moves
        std::fs::write(old.join("Jane Doe.txt"), b"b\n").unwrap();
        std::fs::write(old.join("Bob.txt"), b"d\n").unwrap();
        let refused = w
            .relocate(old.clone(), new.clone())
            .expect("queued")
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert!(refused.is_err());
        assert_eq!(std::fs::read(old.join("Jane Doe.txt")).unwrap(), b"b\n");
        assert_eq!(std::fs::read(old.join("Bob.txt")).unwrap(), b"d\n");
        assert!(!new.join("Bob.txt").exists());
        assert_eq!(std::fs::read(new.join("Jane Doe.txt")).unwrap(), b"a\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_log_that_cannot_move_undoes_the_move() {
        let dir = temp();
        let (old, new) = (dir.join("old"), dir.join("new"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("Jane Doe.txt"), b"a\n").unwrap();
        std::fs::write(old.join(super::super::conversations::FILE), b"log").unwrap();
        // a folder where the log should land: it cannot be replaced
        std::fs::create_dir_all(new.join(super::super::conversations::FILE)).unwrap();
        let w = Worker::start().unwrap();
        let moved = w
            .relocate(old.clone(), new.clone())
            .expect("queued")
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert!(moved.is_err());
        assert_eq!(std::fs::read(old.join("Jane Doe.txt")).unwrap(), b"a\n");
        assert_eq!(std::fs::read(old.join(super::super::conversations::FILE)).unwrap(), b"log");
        assert!(!new.join("Jane Doe.txt").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn combines_candidate_files_by_time_without_duplicates_and_keeps_undated_order() {
        let dir = temp();
        let f1 = dir.join("Jane Doe.txt");
        let f2 = dir.join("jane_doe.txt");
        let c1 = "[2026/10/12 14:00]  Jane Doe: first\n\
                  [14:01]  Jane Doe: undated a\n\
                  [14:02]  Jane Doe: undated b\n\
                  [2026/10/12 14:05]  Jane Doe: shared line\n\
                  [2026/10/12 14:05]  Jane Doe: repeated twice\n\
                  [2026/10/12 14:05]  Jane Doe: repeated twice\n\
                  [2026/10/12 14:10]  Jane Doe: last\n";
        let c2 = "[2026/10/12 14:03]  Jane Doe: between\n\
                  [2026/10/12 14:05:45]  Jane Doe: shared line\n\
                  [2026/10/12 14:05]  Jane Doe: repeated twice\n\
                  [14:06]  Jane Doe: undated c\n";
        std::fs::write(&f1, c1).unwrap();
        std::fs::write(&f2, c2).unwrap();

        let entries = read_combined_tail(&[f1.clone(), f2.clone()]);
        let texts: Vec<&str> = entries.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "first",
                "undated a",
                "undated b",
                "between",
                "shared line",
                "repeated twice",
                "repeated twice",
                "undated c",
                "last",
            ]
        );
        // Neither file on disk was modified
        assert_eq!(std::fs::read_to_string(&f1).unwrap(), c1);
        assert_eq!(std::fs::read_to_string(&f2).unwrap(), c2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A file's modification time set to `secs` after the epoch.
    fn set_modified(path: &Path, secs: u64) {
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(secs)).unwrap();
    }

    #[test]
    fn time_only_lines_are_never_merged_and_their_files_follow_their_time() {
        let dir = temp();
        // two days' files with times only (LogTimestampDate off), the
        // newer one first in the candidates' order
        let newer = dir.join("Jane Doe.txt");
        let older = dir.join("jane_doe.txt");
        std::fs::write(&newer, "[14:05]  Jane Doe: same\n[14:06]  Jane Doe: from the newer day\n").unwrap();
        std::fs::write(&older, "[14:05]  Jane Doe: same\n[14:06]  Jane Doe: from the older day\n").unwrap();
        set_modified(&older, 1_790_000_000);
        set_modified(&newer, 1_793_000_000);
        // a dated file between them, its line written twice to two files
        let dated = dir.join("Jane Doe-2026-10.txt");
        let dated_user = dir.join("jane_doe-2026-10.txt");
        std::fs::write(&dated, "[2026/10/12 14:05]  Jane Doe: dated\n").unwrap();
        std::fs::write(&dated_user, "[2026/10/12 14:05]  Jane Doe: dated\n").unwrap();
        let entries = read_combined_tail(&[newer, dated, older, dated_user]);
        let texts: Vec<&str> = entries.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["same", "from the older day", "dated", "same", "from the newer day"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_tail_reads_the_newest_files_only() {
        let dir = temp();
        let mut paths = Vec::new();
        for month in 1..=6u64 {
            let path = dir.join(format!("Jane Doe-2026-{month:02}.txt"));
            std::fs::write(&path, format!("[2026/{month:02}/12 14:05]  Jane Doe: month {month}\n")).unwrap();
            set_modified(&path, 1_767_000_000 + month * 2_600_000);
            paths.push(path);
        }
        let entries = read_combined_tail(&paths);
        let texts: Vec<&str> = entries.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["month 3", "month 4", "month 5", "month 6"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_shared_target_is_resolved_again_on_a_new_day_or_when_its_file_is_gone() {
        let target = |epoch| SharedAppendTarget {
            epoch,
            dir: PathBuf::from("shared"),
            peer: Some(Uuid::from_u128(7)),
            stem: "Jane Doe".into(),
            today: None,
            scheme: super::super::names::NamingScheme::Legacy,
        };
        let mut targets = SharedTargets::default();
        let mut resolved = 0;
        let mut resolve = |name: &str| {
            resolved += 1;
            Some(PathBuf::from(name))
        };
        let day = (2026, 10, 31);
        let there = |_: &Path| true;
        assert_eq!(targets.target(&target(1), day, there, || resolve("a")), Some(PathBuf::from("a")));
        // the same day, the file still there: no listing of the folder
        assert_eq!(targets.target(&target(1), day, there, || resolve("b")), Some(PathBuf::from("a")));
        // a new day: the file the other viewer started for it is followed
        assert_eq!(
            targets.target(&target(1), (2026, 11, 1), there, || resolve("c")),
            Some(PathBuf::from("c"))
        );
        // the file gone: resolved again
        assert_eq!(
            targets.target(&target(1), (2026, 11, 1), |_| false, || resolve("d")),
            Some(PathBuf::from("d"))
        );
        // another epoch (login, folder change): resolved again
        assert_eq!(
            targets.target(&target(2), (2026, 11, 1), there, || resolve("e")),
            Some(PathBuf::from("e"))
        );
        assert_eq!(resolved, 4);
    }
}
