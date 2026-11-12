//! Logging to the console and to `aurora.log` in the logs directory (the
//! previous run is kept as `aurora.previous.log`), so a session can be
//! shared after a test even when the viewer was started without a console.
//! Capability URLs are secrets for the session: their id is masked.
//! Each start deletes, on a thread of its own, the viewer's log files past
//! the newest [`KEEP_NEWEST`] that nothing wrote to for [`KEEP_FOR`], never
//! this run's nor one another viewer still has open ([`prune_old_logs`]).
//!
//! A thread that logs never waits for the disk (#2655): the line goes into
//! a bounded queue, and the "aurora-log-write" thread writes it to the
//! files and the console, in the order logged. A disk that stalls holds
//! that thread alone; when the queue is full, the lines are counted and
//! dropped, and a "N log lines dropped" line marks the gap once room
//! returns. A panic and the normal exit wait up to [`DRAIN_WAIT`] for the
//! queue to reach the files. A native crash (access violation, signal)
//! does not wait: it may lose the lines still queued, a few microseconds'
//! worth while the disk keeps up, up to the whole queue while it stalls.
//!
//! The writer flushes the files to the disk at most once per second: a PC
//! that freezes and is reset keeps the log up to the second before. Left to
//! the system, ext4 brought the whole log back empty after a reset in the
//! first half minute of a session (#1809).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant, SystemTime};

/// Time between two flushes of the log files to the disk.
const FLUSH_PERIOD: Duration = Duration::from_secs(1);

/// Lines the queue holds while the disk lags (a few MiB at most); past
/// that, they are dropped rather than waited for.
const QUEUE_LINES: usize = 8192;

/// Bytes the writer gathers into one write when lines arrive in a burst.
const BATCH_BYTES: usize = 64 * 1024;

/// Longest wait for the queue to reach the files at a panic and at exit.
pub const DRAIN_WAIT: Duration = Duration::from_millis(500);

pub fn logs_dir() -> PathBuf {
    if let Some(test) = aurora_updater::test_environment() {
        return test.root.join("logs");
    }
    directories::ProjectDirs::from("org", "Aurora", "AuroraViewer")
        .map(|d| d.data_local_dir().join("logs"))
        .unwrap_or_else(|| PathBuf::from("logs"))
}

pub fn log_path() -> PathBuf {
    logs_dir().join(format!("{}.log", log_stem()))
}

/// The offline demo logs apart, so it never rotates a live session's log;
/// `--title` adds the window name (one log per test window) and
/// AURORA_LOG_NAME picks any other name.
fn log_stem() -> String {
    if let Some(name) = std::env::var_os("AURORA_LOG_NAME").filter(|n| !n.is_empty()) {
        return name.to_string_lossy().into_owned();
    }
    let base = if std::env::var_os("AURORA_DEMO").is_some() {
        "aurora-demo"
    } else {
        "aurora"
    };
    match crate::cli::title_slug() {
        Some(slug) => format!("{base}-{slug}"),
        None => format!("{base}-{}", std::process::id()),
    }
}

/// Show the logs directory in the system file manager.
pub fn open_logs_dir() {
    let dir = logs_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Err(e) = open_directory(&dir) {
        log::warn!("cannot open {}: {e}", dir.display());
    }
}

/// Show a directory in the system file manager (also the transcripts folder).
pub fn open_directory(path: &Path) -> std::io::Result<()> {
    crate::open_path::open(path)
}

/// Mask the id of capability URLs (".../cap/<uuid>").
fn redact(line: &str) -> std::borrow::Cow<'_, str> {
    if !line.contains("/cap/") {
        return line.into();
    }
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find("/cap/") {
        out.push_str(&rest[..i + 5]);
        rest = &rest[i + 5..];
        let end = rest.find(|c: char| !(c.is_ascii_hexdigit() || c == '-')).unwrap_or(rest.len());
        if end > 0 {
            out.push('…');
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out.into()
}

thread_local! {
    /// What this thread logs while a capture runs (`capture`).
    static CAPTURE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Run `f` and return, besides its result, the lines it logged on this
/// thread without their timestamp and target (the diagnostic copied to the
/// clipboard, scene/describe.rs). env_logger writes in the calling thread.
pub fn capture<R>(f: impl FnOnce() -> R) -> (R, String) {
    CAPTURE.with(|c| *c.borrow_mut() = Some(String::new()));
    let result = f();
    let text = CAPTURE.with(|c| c.borrow_mut().take()).unwrap_or_default();
    (result, strip_headers(&text))
}

/// Log lines without env_logger's header
/// ("[2026-10-20T12:00:00.000Z INFO  aurora_viewer::scene] text").
fn strip_headers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let line = match line.strip_prefix('[').and_then(|l| l.split_once("] ")) {
            Some((_, rest)) => rest,
            None => line,
        };
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// A line the Dev Toolbox keeps in memory: J3 and L3 show the last warnings,
/// J5 the recent output of a diagnostic, without reading the log file.
#[derive(Debug, Clone)]
pub struct Recent {
    pub at: std::time::Instant,
    /// "HH:MM:SS" of the record's header (UTC, as in the log).
    pub clock: String,
    pub level: log::Level,
    pub text: String,
}

const WARNINGS_KEPT: usize = 200;
const TOOL_LINES_KEPT: usize = 600;

/// The diagnostics whose output J5 copies, by the start of their lines.
pub const TOOL_PREFIXES: &[(&str, &str)] = &[
    ("P3", "slow sync:"),
    ("P4", "still check:"),
    ("P5", "sync check:"),
    ("P6", "perf summary:"),
    ("P6", "net summary:"),
    ("O9", "sculpt watch:"),
    ("M1", "media objects:"),
];

static WARNINGS: std::sync::Mutex<std::collections::VecDeque<Recent>> = std::sync::Mutex::new(std::collections::VecDeque::new());
static TOOL_LINES: std::sync::Mutex<std::collections::VecDeque<(&'static str, Recent)>> =
    std::sync::Mutex::new(std::collections::VecDeque::new());

/// The header of a record: (clock, level, the message after the header).
fn parse_record(text: &str) -> Option<(&str, log::Level, &str)> {
    let rest = text.strip_prefix('[')?;
    let (header, message) = rest.split_once("] ")?;
    let mut parts = header.split_whitespace();
    let stamp = parts.next()?;
    let level = parts.next()?.parse().ok()?;
    let clock = stamp.split_once('T').map_or(stamp, |(_, t)| t.get(..8).unwrap_or(t));
    Some((clock, level, message.trim_end()))
}

/// Keep what the Dev Toolbox shows from one record.
fn remember(text: &str) {
    let Some((clock, level, message)) = parse_record(text) else {
        return;
    };
    let recent = || Recent {
        at: std::time::Instant::now(),
        clock: clock.to_owned(),
        level,
        text: message.to_owned(),
    };
    if level <= log::Level::Warn
        && let Ok(mut ring) = WARNINGS.lock()
    {
        if ring.len() == WARNINGS_KEPT {
            ring.pop_front();
        }
        ring.push_back(recent());
    }
    if let Some((tool, _)) = TOOL_PREFIXES.iter().find(|(_, prefix)| message.starts_with(prefix))
        && let Ok(mut ring) = TOOL_LINES.lock()
    {
        if ring.len() == TOOL_LINES_KEPT {
            ring.pop_front();
        }
        ring.push_back((tool, recent()));
    }
}

/// The last `n` warnings and errors, the oldest first.
pub fn recent_warnings(n: usize) -> Vec<Recent> {
    WARNINGS
        .lock()
        .map(|ring| ring.iter().skip(ring.len().saturating_sub(n)).cloned().collect())
        .unwrap_or_default()
}

/// The lines the diagnostics of [`TOOL_PREFIXES`] wrote in the last `since`.
pub fn tool_lines(since: std::time::Duration) -> Vec<(&'static str, Recent)> {
    let now = std::time::Instant::now();
    TOOL_LINES
        .lock()
        .map(|ring| ring.iter().filter(|(_, r)| now.duration_since(r.at) <= since).cloned().collect())
        .unwrap_or_default()
}

/// Where the writer thread puts the lines: a log file or the console.
trait LogSink: Send {
    fn put(&mut self, bytes: &[u8]);
    /// Make what was put survive a reset of the PC.
    fn sync(&mut self) {}
}

struct LogFile(std::fs::File);

impl LogSink for LogFile {
    fn put(&mut self, bytes: &[u8]) {
        let _ = self.0.write_all(bytes);
        crate::disk_io::written(crate::disk_io::Origin::Logs, bytes.len());
    }

    fn sync(&mut self) {
        let _ = self.0.sync_data();
    }
}

/// stderr may block too (a pipe nobody reads): it is the writer's as well.
struct Console;

impl LogSink for Console {
    fn put(&mut self, bytes: &[u8]) {
        let mut stderr = std::io::stderr().lock();
        let _ = stderr.write_all(bytes);
        let _ = stderr.flush();
    }
}

enum Message {
    Line(String),
    /// Lines dropped while the queue was full, before the next one.
    Dropped(u64),
}

fn dropped_line(count: u64) -> String {
    format!("[log] {count} log lines dropped: the disk did not keep up\n")
}

/// How far the writer has got, for [`Queue::drain`].
#[derive(Default)]
struct Progress {
    written: Mutex<u64>,
    advanced: Condvar,
}

/// The bounded queue in front of the writer thread.
struct Queue {
    sender: SyncSender<Message>,
    /// Lines dropped since the last marker.
    dropped: AtomicU64,
    /// Messages queued since the start.
    sent: AtomicU64,
    progress: Arc<Progress>,
    writer: std::thread::ThreadId,
}

impl Queue {
    /// Start the writer thread with `sinks`. On failure the sinks come back,
    /// for the caller to write to them itself.
    fn start(sinks: Vec<Box<dyn LogSink>>, lines: usize) -> Result<Self, Vec<Box<dyn LogSink>>> {
        let (sender, receiver) = std::sync::mpsc::sync_channel(lines);
        let progress = Arc::new(Progress::default());
        // the closure is dropped when the thread cannot start: the sinks
        // stay reachable from here
        let slot = Arc::new(Mutex::new(Some(sinks)));
        let (thread_slot, thread_progress) = (slot.clone(), progress.clone());
        let spawned = std::thread::Builder::new().name("aurora-log-write".into()).spawn(move || {
            let sinks = thread_slot
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
                .unwrap_or_default();
            write_lines(&receiver, sinks, &thread_progress);
        });
        match spawned {
            Ok(handle) => Ok(Self {
                sender,
                dropped: AtomicU64::new(0),
                sent: AtomicU64::new(0),
                progress,
                writer: handle.thread().id(),
            }),
            Err(e) => {
                eprintln!("log writer not started, the lines are written as logged: {e}");
                Err(slot.lock().unwrap_or_else(PoisonError::into_inner).take().unwrap_or_default())
            }
        }
    }

    /// Queue `line` without ever waiting: a full queue drops it.
    fn push(&self, line: String) {
        let dropped = self.dropped.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            if self.sender.try_send(Message::Dropped(dropped)).is_err() {
                self.dropped.fetch_add(dropped + 1, Ordering::Relaxed);
                return;
            }
            self.sent.fetch_add(1, Ordering::Release);
        }
        match self.sender.try_send(Message::Line(line)) {
            Ok(()) => {
                self.sent.fetch_add(1, Ordering::Release);
            }
            Err(TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            // the writer is gone (it panicked): nothing writes any more
            Err(TrySendError::Disconnected(_)) => {}
        }
    }

    /// Wait at most `timeout` for what was queued so far to reach the
    /// sinks; false when it did not (a stalled disk, or the writer itself).
    fn drain(&self, timeout: Duration) -> bool {
        if std::thread::current().id() == self.writer {
            return false;
        }
        let target = self.sent.load(Ordering::Acquire);
        let deadline = Instant::now() + timeout;
        let mut written = self.progress.written.lock().unwrap_or_else(PoisonError::into_inner);
        while *written < target {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            written = self
                .progress
                .advanced
                .wait_timeout(written, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }
}

/// The writer thread: the lines in the order queued, the gathered burst in
/// one write per sink, and the sinks synced at most once per `FLUSH_PERIOD`
/// after a write. Logs nothing itself.
fn write_lines(receiver: &Receiver<Message>, mut sinks: Vec<Box<dyn LogSink>>, progress: &Progress) {
    let mut written = 0u64;
    let mut unsynced = false;
    let mut synced = Instant::now();
    let mut batch = String::new();
    loop {
        let first = if unsynced {
            match receiver.recv_timeout(FLUSH_PERIOD.saturating_sub(synced.elapsed())) {
                Ok(message) => Some(message),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match receiver.recv() {
                Ok(message) => Some(message),
                Err(_) => break,
            }
        };
        if let Some(first) = first {
            batch.clear();
            let mut next = Some(first);
            while let Some(message) = next {
                match message {
                    Message::Line(line) => batch.push_str(&line),
                    Message::Dropped(count) => batch.push_str(&dropped_line(count)),
                }
                written += 1;
                next = if batch.len() < BATCH_BYTES {
                    receiver.try_recv().ok()
                } else {
                    None
                };
            }
            for sink in &mut sinks {
                sink.put(batch.as_bytes());
            }
            *progress.written.lock().unwrap_or_else(PoisonError::into_inner) = written;
            progress.advanced.notify_all();
            unsynced = true;
        }
        if unsynced && synced.elapsed() >= FLUSH_PERIOD {
            for sink in &mut sinks {
                sink.sync();
            }
            unsynced = false;
            synced = Instant::now();
        }
    }
    for sink in &mut sinks {
        sink.sync();
    }
}

/// The queue of the process's log, for [`drain`].
static QUEUE: OnceLock<Queue> = OnceLock::new();

/// Wait at most `timeout` for the lines logged so far to reach the log
/// files (at a panic, at exit). False when they did not.
pub fn drain(timeout: Duration) -> bool {
    QUEUE.get().is_none_or(|queue| queue.drain(timeout))
}

/// env_logger's target. env_logger writes each record with one call, in the
/// thread that logs and under its own lock (so the queue keeps their order).
enum Tee {
    Queued(&'static Queue),
    /// The writer thread could not start: written as logged.
    Direct(Vec<Box<dyn LogSink>>),
}

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        let text = redact(&text);
        CAPTURE.with(|c| {
            if let Some(s) = c.borrow_mut().as_mut() {
                s.push_str(&text);
            }
        });
        remember(&text);
        match self {
            Self::Queued(queue) => queue.push(text.into_owned()),
            Self::Direct(sinks) => {
                for sink in sinks {
                    sink.put(text.as_bytes());
                }
            }
        }
        Ok(buf.len())
    }

    /// Nothing to flush here: the writer thread writes.
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Log files kept whatever their age: the latest sessions.
const KEEP_NEWEST: usize = 20;
/// Log files written to this recently are kept whatever their number: about
/// two releases' worth of sessions, and the log of a viewer that is still
/// running (each viewer writes a file of its own, [`log_stem`]).
const KEEP_FOR: Duration = Duration::from_secs(14 * 24 * 60 * 60);

/// A log file of the viewer in the logs folder: `aurora-<pid>.log`,
/// `aurora-<title>.log`, `aurora-demo-….log`, their `.previous.log`, and
/// the older `aurora.log`. A name picked with AURORA_LOG_NAME, the crash
/// sessions' folder and the other files of the folder are not.
fn is_viewer_log(name: &str) -> bool {
    name.starts_with("aurora") && name.ends_with(".log")
}

/// The log files to delete among `files` (path, last write): past the
/// [`KEEP_NEWEST`] newest, those nothing wrote to for [`KEEP_FOR`]. The
/// files of this run (`keep`) are never chosen.
fn prune_choice(mut files: Vec<(PathBuf, SystemTime)>, now: SystemTime, keep: &[PathBuf]) -> Vec<PathBuf> {
    files.retain(|(path, _)| !keep.contains(path));
    files.sort_by(|a, b| b.1.cmp(&a.1));
    files
        .into_iter()
        .skip(KEEP_NEWEST)
        .filter(|(_, written)| now.duration_since(*written).is_ok_and(|age| age > KEEP_FOR))
        .map(|(path, _)| path)
        .collect()
}

/// Whether another process still holds the file open. Windows refuses to
/// open a file with no sharing while any handle on it is open; on Unix a
/// running viewer keeps a shared advisory lock on its log (`init`). A file
/// that cannot be opened at all counts as in use.
#[cfg(windows)]
fn in_use(path: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().read(true).share_mode(0).open(path).is_err()
}

#[cfg(unix)]
fn in_use(path: &Path) -> bool {
    match std::fs::File::open(path) {
        Ok(file) => file.try_lock().is_err(),
        Err(_) => true,
    }
}

#[cfg(not(any(windows, unix)))]
fn in_use(_: &Path) -> bool {
    true
}

/// Delete the old log files of `dir` ([`prune_choice`]) on a thread of its
/// own, so a folder of many files on a slow disk never delays the start. A
/// file still in use ([`in_use`]) or that cannot be deleted is left, for a
/// later start.
fn prune_old_logs(dir: PathBuf, keep: Vec<PathBuf>) {
    let spawned = std::thread::Builder::new().name("aurora-log-prune".into()).spawn(move || {
        let Ok(entries) = std::fs::read_dir(&dir) else { return };
        let files = entries
            .flatten()
            .filter(|entry| entry.file_name().to_str().is_some_and(is_viewer_log))
            .filter_map(|entry| {
                let meta = entry.metadata().ok().filter(std::fs::Metadata::is_file)?;
                Some((entry.path(), meta.modified().ok()?))
            })
            .collect();
        let removed = prune_choice(files, SystemTime::now(), &keep)
            .into_iter()
            .filter(|path| !in_use(path) && std::fs::remove_file(path).is_ok())
            .count();
        if removed > 0 {
            log::info!("{removed} old log files removed from {}", dir.display());
        }
    });
    if let Err(e) = spawned {
        log::debug!("old log files left: {e}");
    }
}

pub fn init() {
    let dir = logs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = log_path();
    let previous = dir.join(format!("{}.previous.log", log_stem()));
    let _ = std::fs::rename(&path, &previous);
    let mut sinks: Vec<Box<dyn LogSink>> = vec![Box::new(Console)];
    let own = std::fs::File::create(&path).ok();
    // Marks this log as in use for another viewer's `prune_old_logs` (Unix
    // locks are advisory: the writes are not held back). Windows tells an
    // open file apart without it.
    #[cfg(unix)]
    {
        if let Some(file) = &own {
            let _ = file.try_lock_shared();
        }
    }
    for file in [own, crate::crash::session_log()].into_iter().flatten() {
        sinks.push(Box::new(LogFile(file)));
    }
    let tee = match Queue::start(sinks, QUEUE_LINES) {
        Ok(queue) => Tee::Queued(QUEUE.get_or_init(|| queue)),
        Err(sinks) => Tee::Direct(sinks),
    };
    // `pulseaudio` off: the PulseAudio client logs `ERROR Reactor error:
    // Client disconnected` each time cpal closes a stream (every output or
    // capture device switch). Real stream failures reach aurora-audio through
    // cpal's error callback, which reopens the device.
    env_logger::Builder::from_env(
        env_logger::Env::default()
            .default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn,reqwest=warn,symphonia=warn,pulseaudio=off,zbus=warn"),
    )
    .format_timestamp_millis()
    .target(env_logger::Target::Pipe(Box::new(tee)))
    .init();
    std::panic::set_hook(Box::new(|info| {
        // wgpu's panic for a device lost during a wait, caught by the
        // renderer and taken as the loss (aurora-render device_lost.rs);
        // any other panic there is reported
        if aurora_render::in_guarded_poll() && info.payload_as_str().is_some_and(aurora_render::is_loss_panic) {
            log::warn!("wgpu: {info}");
            return;
        }
        let bt = std::backtrace::Backtrace::force_capture();
        crate::crash::panic(info, &bt);
        log::error!("panic: {info}\n{bt}");
        // the process may end right after (a panic in main, a critical
        // thread's death): the lines before it reach the files first
        drain(DRAIN_WAIT);
    }));
    log::info!(
        "Aurora Viewer {} starting ({} {}, {} allocator), log: {}",
        aurora_version::version(),
        std::env::consts::OS,
        std::env::consts::ARCH,
        crate::allocator::name(),
        path.display()
    );
    prune_old_logs(dir, vec![path, previous]);
}

#[cfg(test)]
mod tests {
    use super::{KEEP_FOR, KEEP_NEWEST, LogSink, Queue, dropped_line, is_viewer_log, parse_record, prune_choice, redact, strip_headers};
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// What the writer put, as text.
    #[derive(Clone, Default)]
    struct Recorded(Arc<Mutex<String>>);

    impl Recorded {
        fn lines(&self) -> Vec<String> {
            self.0.lock().unwrap().lines().map(str::to_owned).collect()
        }
    }

    impl LogSink for Recorded {
        fn put(&mut self, bytes: &[u8]) {
            self.0.lock().unwrap().push_str(std::str::from_utf8(bytes).unwrap());
        }
    }

    /// A disk that stalls: each write tells it began, then waits for leave.
    struct Stalled {
        record: Recorded,
        began: Sender<()>,
        leave: Arc<Mutex<Receiver<()>>>,
    }

    impl LogSink for Stalled {
        fn put(&mut self, bytes: &[u8]) {
            let _ = self.began.send(());
            let _ = self.leave.lock().unwrap().recv();
            self.record.put(bytes);
        }
    }

    fn line(n: usize) -> String {
        format!("line {n}\n")
    }

    #[test]
    fn a_stalled_disk_drops_lines_without_waiting_and_marks_the_gap() {
        let record = Recorded::default();
        let (began_tx, began) = channel();
        let (leave, leave_rx) = channel();
        let sink = Stalled {
            record: record.clone(),
            began: began_tx,
            leave: Arc::new(Mutex::new(leave_rx)),
        };
        let queue = Queue::start(vec![Box::new(sink)], 4).ok().unwrap();
        queue.push(line(0));
        // the writer holds line 0 in the stalled write: the queue is empty
        began.recv_timeout(Duration::from_secs(10)).unwrap();
        let start = Instant::now();
        for n in 1..=7 {
            queue.push(line(n));
        }
        // 4 fit, 3 are dropped, and nothing waited for the disk
        assert!(start.elapsed() < Duration::from_secs(1), "pushes waited {:?}", start.elapsed());
        assert!(!queue.drain(Duration::from_millis(50)), "drained while the disk stalls");
        for _ in 0..16 {
            let _ = leave.send(());
        }
        assert!(queue.drain(Duration::from_secs(10)));
        queue.push(line(8));
        assert!(queue.drain(Duration::from_secs(10)));
        let marker = dropped_line(3);
        assert_eq!(
            record.lines(),
            ["line 0", "line 1", "line 2", "line 3", "line 4", marker.trim_end(), "line 8"]
        );
    }

    #[test]
    fn lines_keep_their_order() {
        let record = Recorded::default();
        // room for every line: none is dropped
        let queue = Arc::new(Queue::start(vec![Box::new(record.clone())], 2000).ok().unwrap());
        let threads: Vec<_> = (0..4)
            .map(|t| {
                let queue = queue.clone();
                std::thread::spawn(move || {
                    for n in 0..500 {
                        queue.push(format!("{t} {n}\n"));
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert!(queue.drain(Duration::from_secs(10)));
        let lines = record.lines();
        assert_eq!(lines.len(), 2000);
        for t in 0..4 {
            let mine: Vec<usize> = lines
                .iter()
                .filter_map(|l| l.split_once(' ').filter(|(who, _)| *who == t.to_string()))
                .map(|(_, n)| n.parse().unwrap())
                .collect();
            assert_eq!(mine, (0..500).collect::<Vec<_>>(), "thread {t}");
        }
    }

    #[test]
    fn drain_waits_for_a_slow_disk() {
        struct Slow(Recorded);
        impl LogSink for Slow {
            fn put(&mut self, bytes: &[u8]) {
                std::thread::sleep(Duration::from_millis(2));
                self.0.put(bytes);
            }
        }
        let record = Recorded::default();
        let queue = Queue::start(vec![Box::new(Slow(record.clone()))], 256).ok().unwrap();
        for n in 0..100 {
            queue.push(line(n));
        }
        assert!(queue.drain(Duration::from_secs(10)));
        assert_eq!(record.lines(), (0..100).map(|n| format!("line {n}")).collect::<Vec<_>>());
    }

    /// The time a thread that logs spends per line, written to two files as
    /// before (#2655) and queued for the writer as now. Run with
    /// `cargo test -p aurora-viewer logging::tests::cost_per_line -- --ignored --nocapture`.
    #[test]
    #[ignore = "measure"]
    fn cost_per_line() {
        use std::io::Write;
        const LINES: usize = 20_000;
        let dir = std::env::temp_dir().join(format!("aurora-log-cost-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let text = "[2026-11-06T12:00:00.000Z INFO  aurora_viewer::media] media objects: 12 faces, 3 players, 1 stream\n";
        let mut files = [0, 1].map(|n| std::fs::File::create(dir.join(format!("direct-{n}.log"))).unwrap());
        let start = Instant::now();
        for _ in 0..LINES {
            for file in &mut files {
                file.write_all(text.as_bytes()).unwrap();
            }
        }
        let direct = start.elapsed();
        let sinks: Vec<Box<dyn LogSink>> = (0..2)
            .map(|n| Box::new(super::LogFile(std::fs::File::create(dir.join(format!("queued-{n}.log"))).unwrap())) as Box<dyn LogSink>)
            .collect();
        let queue = Queue::start(sinks, super::QUEUE_LINES).ok().unwrap();
        let start = Instant::now();
        for _ in 0..LINES {
            queue.push(text.to_owned());
        }
        let queued = start.elapsed();
        assert!(queue.drain(Duration::from_secs(30)));
        drop(queue);
        let per = |d: Duration| d.as_nanos() / LINES as u128;
        println!("per line: direct {} ns, queued {} ns", per(direct), per(queued));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn record_headers_give_clock_and_level() {
        let (clock, level, message) =
            parse_record("[2026-10-20T02:34:46.538Z WARN  aurora_viewer::scene::textures] texture memory: pressure\n").unwrap();
        assert_eq!(clock, "02:34:46");
        assert_eq!(level, log::Level::Warn);
        assert_eq!(message, "texture memory: pressure");
        assert!(parse_record("  continuation line").is_none());
    }

    #[test]
    fn captured_lines_lose_their_header() {
        let text = "[2026-10-20T12:00:00.000Z INFO  aurora_viewer::scene::describe] attachment '?' point 40\n  prim [0 tex] x\n";
        assert_eq!(strip_headers(text), "attachment '?' point 40\n  prim [0 tex] x\n");
    }

    #[test]
    fn masks_capability_ids() {
        assert_eq!(
            redact(
                "error sending request for url (https://sim1.agni.lindenlab.com:12043/cap/0b6c6a8e-1f0d-4d3e-9a1b-2c3d4e5f6a7b): timeout"
            ),
            "error sending request for url (https://sim1.agni.lindenlab.com:12043/cap/…): timeout"
        );
        assert_eq!(redact("plain line"), "plain line");
    }

    #[test]
    fn only_the_viewer_logs_are_pruned() {
        for name in [
            "aurora-1234.log",
            "aurora-1234.previous.log",
            "aurora-demo-test-x.log",
            "aurora.log",
        ] {
            assert!(is_viewer_log(name), "{name}");
        }
        for name in ["resource-monitor_2026.csv", "crashes", "mine.log", "aurora-1234.log.txt"] {
            assert!(!is_viewer_log(name), "{name}");
        }
    }

    /// The newest files stay whatever their age, the recent ones whatever
    /// their number, and this run's files always.
    #[test]
    fn old_logs_past_the_newest_are_chosen() {
        use std::path::PathBuf;
        use std::time::SystemTime;
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1 << 32);
        let day = Duration::from_secs(24 * 60 * 60);
        let file = |n: usize, age: Duration| (PathBuf::from(format!("aurora-{n}.log")), now - age);
        // fewer than KEEP_NEWEST, all old: nothing goes
        let few: Vec<_> = (0..KEEP_NEWEST).map(|n| file(n, KEEP_FOR * 4)).collect();
        assert!(prune_choice(few, now, &[]).is_empty());
        // many recent ones: nothing goes either
        let recent: Vec<_> = (0..KEEP_NEWEST * 3).map(|n| file(n, day)).collect();
        assert!(prune_choice(recent, now, &[]).is_empty());
        // the newest KEEP_NEWEST stay, then only the old ones go
        let hour = Duration::from_secs(60 * 60);
        let mut files: Vec<_> = (0..KEEP_NEWEST).map(|n| file(n, hour * n as u32)).collect();
        files.push(file(100, KEEP_FOR * 3));
        files.push(file(101, KEEP_FOR - day));
        files.push(file(102, KEEP_FOR * 5));
        let mut chosen = prune_choice(files.clone(), now, &[]);
        chosen.sort();
        assert_eq!(chosen, vec![PathBuf::from("aurora-100.log"), PathBuf::from("aurora-102.log")]);
        // this run's files never, even the oldest
        let keep = [PathBuf::from("aurora-102.log")];
        assert_eq!(prune_choice(files, now, &keep), vec![PathBuf::from("aurora-100.log")]);
        // a clock set back (a write "in the future") keeps the file
        let ahead: Vec<_> = (0..KEEP_NEWEST + 1)
            .map(|n| (PathBuf::from(format!("aurora-{n}.log")), now + day))
            .collect();
        assert!(prune_choice(ahead, now, &[]).is_empty());
    }
}
