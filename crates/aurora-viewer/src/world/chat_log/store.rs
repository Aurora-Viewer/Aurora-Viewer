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

enum Job {
    Append {
        path: PathBuf,
        text: String,
    },
    Tail {
        dir: PathBuf,
        stem: String,
        today: Option<(i64, i64, i64)>,
        conversation: Uuid,
        epoch: u64,
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
    Sync(Sender<()>),
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

    fn send(&self, job: Job) {
        match self.jobs.try_send(job) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                if !self.overflow.swap(true, Ordering::Relaxed) {
                    log::warn!("conversation transcripts: the disk does not keep up, some lines are not written");
                }
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }

    /// Append text (whole lines, with their ends of line) to a file.
    pub fn append(&self, path: PathBuf, text: String) {
        self.send(Job::Append { path, text });
    }

    pub fn tail(&self, dir: PathBuf, stem: String, today: Option<(i64, i64, i64)>, conversation: Uuid, epoch: u64) {
        self.send(Job::Tail {
            dir,
            stem,
            today,
            conversation,
            epoch,
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
    /// queued; the answer is the number of files moved.
    pub fn relocate(&self, from: PathBuf, to: PathBuf) -> Receiver<std::io::Result<usize>> {
        let (done, rx) = crossbeam_channel::bounded(1);
        self.send(Job::Move { from, to, done });
        rx
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

fn run(jobs: Receiver<Job>, loaded: Sender<Loaded>) {
    // a failure is said once per run, without the file name (it is the name
    // of a resident or a group, and the viewer's log may be shared)
    let mut failed = false;
    let mut next = jobs.recv().ok();
    while let Some(job) = next.take() {
        match job {
            Job::Append { path, mut text } => {
                // the lines already waiting for the same file go with it
                loop {
                    match jobs.try_recv() {
                        Ok(Job::Append { path: p, text: t }) if p == path => text.push_str(&t),
                        Ok(other) => {
                            next = Some(other);
                            break;
                        }
                        Err(_) => break,
                    }
                }
                if let Err(e) = append(&path, text.as_bytes())
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
            } => {
                let entries = super::names::existing_file(&dir, &stem, today)
                    .and_then(|path| read_tail(&path).ok())
                    .unwrap_or_default();
                let _ = loaded.send(Loaded {
                    conversation,
                    epoch,
                    entries,
                });
            }
            Job::Erase { dir, done } => {
                let _ = done.send(erase(&dir));
            }
            Job::Move { from, to, done } => {
                let _ = done.send(relocate(&from, &to));
            }
            Job::Sync(done) => {
                let _ = done.send(());
            }
        }
        if next.is_none() {
            next = jobs.recv().ok();
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
/// Only `.txt` files directly in it: nothing else is ever touched.
fn erase(dir: &Path) -> std::io::Result<usize> {
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
    let _ = std::fs::remove_dir(from);
    Ok(moved.len())
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
        let moved = w.relocate(old.clone(), new.clone()).recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(moved.unwrap(), 1);
        assert_eq!(std::fs::read(new.join("Jane Doe.txt")).unwrap(), b"a\n");
        assert!(old.join("keep.json").exists(), "only transcripts move");
        // a transcript of the same name in the target: nothing moves
        std::fs::write(old.join("Jane Doe.txt"), b"b\n").unwrap();
        std::fs::write(old.join("Bob.txt"), b"d\n").unwrap();
        let refused = w.relocate(old.clone(), new.clone()).recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(refused.is_err());
        assert_eq!(std::fs::read(old.join("Jane Doe.txt")).unwrap(), b"b\n");
        assert_eq!(std::fs::read(old.join("Bob.txt")).unwrap(), b"d\n");
        assert!(!new.join("Bob.txt").exists());
        assert_eq!(std::fs::read(new.join("Jane Doe.txt")).unwrap(), b"a\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
