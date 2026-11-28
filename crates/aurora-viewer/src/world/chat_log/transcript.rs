//! A whole transcript for the History window, whatever its size: the file
//! is never held in memory. A thread reads it once to note where every
//! hundredth message starts (Firestorm's viewer, LLFloaterConversationPreview,
//! shows pages of CONVERSATION_HISTORY_PAGE_SIZE = 100 messages after
//! loading the whole file in a thread); a page is then read from its offset
//! when it is shown, and a search streams over the file page by page.
//!
//! Memory: one offset per page (8 bytes per 100 messages) and, while a
//! search is on, one match count per page.

use super::format::Entry;
use super::names;
use super::store::{MAX_LINE, Messages, dedup_key, entry_time, line_times, modified_secs};
use crossbeam_channel::{Receiver, Sender};
use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use uuid::Uuid;

/// CONVERSATION_HISTORY_PAGE_SIZE.
pub const PAGE: usize = 100;

/// What a History window opens: either a single file or a conversation whose
/// files (including split Legacy/username/dated variants) are resolved and
/// merged on the `"aurora-transcript"` thread without main-thread disk I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistorySource {
    #[cfg(test)]
    File(PathBuf),
    Conversation {
        dir: PathBuf,
        peer: Option<Uuid>,
        stem: String,
        shared: bool,
    },
}

impl HistorySource {
    /// Folder containing this transcript source.
    pub fn folder(&self) -> Option<&Path> {
        match self {
            #[cfg(test)]
            Self::File(path) => path.parent(),
            Self::Conversation { dir, .. } => Some(dir.as_path()),
        }
    }

    /// The conversation's file stem the source was resolved with.
    pub fn stem(&self) -> Option<&str> {
        match self {
            #[cfg(test)]
            Self::File(_) => None,
            Self::Conversation { stem, .. } => Some(stem.as_str()),
        }
    }
}

/// A position in the file: a message of a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Spot {
    pub page: usize,
    /// Index of the message in its page.
    pub message: usize,
}

enum Cmd {
    Page(usize),
    /// Count the matches of a text (lowercase) in the whole file.
    Search {
        id: u64,
        needle: String,
    },
    /// The match after (or before) a spot; None starts from the end.
    Step {
        id: u64,
        from: Option<Spot>,
        forward: bool,
    },
}

#[derive(Debug)]
pub enum Reply {
    /// The file was read through: its pages. None: it could not be opened.
    Indexed(Option<Index>),
    Page {
        page: usize,
        entries: Vec<Entry>,
    },
    /// Matches counted so far by the search `id`.
    Counted {
        id: u64,
        total: u64,
        done: bool,
    },
    /// Answer to a step: the match, and its rank among all the matches when
    /// the pages before it are counted.
    Found {
        id: u64,
        spot: Option<Spot>,
        rank: Option<u64>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Index {
    pub pages: usize,
    pub messages: u64,
    pub bytes: u64,
}

/// A transcript being read by its thread.
pub struct Transcript {
    source: HistorySource,
    cmds: Sender<Cmd>,
    pub replies: Receiver<Reply>,
    /// Part of the file read by the first pass, in thousandths.
    progress: Arc<AtomicU32>,
}

impl Transcript {
    /// Start reading a single transcript file. None when the thread cannot start.
    #[cfg(test)]
    pub fn open(path: PathBuf) -> Option<Transcript> {
        Self::open_source(HistorySource::File(path))
    }

    /// Start reading a conversation's transcript(s). Candidate files are
    /// resolved and combined on the `"aurora-transcript"` thread.
    pub fn open_source(source: HistorySource) -> Option<Transcript> {
        let (cmds, rx) = crossbeam_channel::unbounded();
        let (tx, replies) = crossbeam_channel::unbounded();
        let progress = Arc::new(AtomicU32::new(0));
        let (src, shared) = (source.clone(), progress.clone());
        std::thread::Builder::new()
            .name("aurora-transcript".into())
            .spawn(move || run(&src, rx, tx, &shared))
            .ok()?;
        Some(Transcript {
            source,
            cmds,
            replies,
            progress,
        })
    }

    pub fn source(&self) -> &HistorySource {
        &self.source
    }

    /// Folder opened by the History toolbar button.
    pub fn folder(&self) -> Option<&Path> {
        self.source.folder()
    }

    /// Whether an import reporting `changed` target stems touched this conversation.
    pub fn matches_changed(&self, changed: &std::collections::BTreeSet<String>) -> bool {
        match &self.source {
            #[cfg(test)]
            HistorySource::File(path) => path.file_stem().and_then(|s| s.to_str()).is_some_and(|s| changed.contains(s)),
            HistorySource::Conversation { stem, shared, .. } => {
                let target_clean = names::clean_file_name(stem.trim());
                let (legacy, username) = if *shared && names::is_peer_stem(stem) {
                    let (l, u) = names::peer_stems(stem);
                    (Some(names::clean_file_name(&l)), u.map(|s| names::clean_file_name(&s)))
                } else {
                    (None, None)
                };
                changed.iter().any(|c| {
                    let undated = names::strip_date_suffix(c);
                    undated.eq_ignore_ascii_case(&target_clean)
                        || legacy.as_deref().is_some_and(|l| undated.eq_ignore_ascii_case(l))
                        || username.as_deref().is_some_and(|u| undated.eq_ignore_ascii_case(u))
                })
            }
        }
    }

    /// How far the first pass is, 0..1.
    pub fn progress(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }

    pub fn page(&self, page: usize) {
        let _ = self.cmds.send(Cmd::Page(page));
    }

    pub fn search(&self, id: u64, needle: String) {
        let _ = self.cmds.send(Cmd::Search { id, needle });
    }

    pub fn step(&self, id: u64, from: Option<Spot>, forward: bool) {
        let _ = self.cmds.send(Cmd::Step { id, from, forward });
    }
}

/// Reads a line, keeping at most `MAX_LINE` bytes of it: the rest of a
/// longer line is skipped. Returns the bytes the line takes in the file; 0
/// at its end.
fn read_line(reader: &mut impl BufRead, line: &mut Vec<u8>) -> std::io::Result<u64> {
    line.clear();
    let mut read = 0u64;
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            return Ok(read);
        }
        let (take, done) = match buf.iter().position(|&b| b == b'\n') {
            Some(i) => (i + 1, true),
            None => (buf.len(), false),
        };
        let room = (MAX_LINE + 2).saturating_sub(line.len());
        line.extend_from_slice(&buf[..take.min(room)]);
        reader.consume(take);
        read += take as u64;
        if done {
            return Ok(read);
        }
    }
}

/// A line that starts a message (not a continuation, not a blank line).
fn starts_message(line: &[u8]) -> bool {
    !matches!(line.first(), None | Some(b' ' | b'\n' | b'\r'))
}

/// First pass: the offset of the first message of every page.
fn index(path: &Path, progress: &AtomicU32) -> std::io::Result<(Vec<u64>, u64, u64)> {
    let file = std::fs::File::open(path)?;
    let bytes = file.metadata()?.len();
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut offsets = Vec::new();
    let (mut at, mut messages) = (0u64, 0u64);
    let mut line = Vec::with_capacity(256);
    loop {
        let before = at;
        let read = read_line(&mut reader, &mut line)?;
        if read == 0 {
            break;
        }
        at += read;
        if starts_message(&line) {
            if messages.is_multiple_of(PAGE as u64) {
                offsets.push(before);
            }
            messages += 1;
        }
        if messages.is_multiple_of(4096) && bytes > 0 {
            progress.store((at.saturating_mul(1000) / bytes) as u32, Ordering::Relaxed);
        }
    }
    progress.store(1000, Ordering::Relaxed);
    Ok((offsets, messages, at.max(bytes)))
}

/// The messages of a page, read from the file kept open by the thread.
fn read_page(file: &mut std::fs::File, offsets: &[u64], page: usize) -> std::io::Result<Vec<Entry>> {
    let Some(&start) = offsets.get(page) else {
        return Ok(Vec::new());
    };
    file.seek(SeekFrom::Start(start))?;
    let len = match offsets.get(page + 1) {
        Some(end) => end.saturating_sub(start),
        None => u64::MAX,
    };
    let mut reader = BufReader::with_capacity(1 << 16, file.take(len));
    let mut messages = Messages::default();
    let mut out = Vec::with_capacity(PAGE);
    let mut line = Vec::with_capacity(256);
    while out.len() <= PAGE && read_line(&mut reader, &mut line)? > 0 {
        out.extend(messages.line(&line));
    }
    out.extend(messages.finish());
    out.truncate(PAGE);
    Ok(out)
}

/// A message contains the searched text (in its sender or its text).
fn hit(e: &Entry, needle: &str) -> bool {
    contains_ci(&e.text, needle) || contains_ci(&e.from, needle)
}

/// Case-insensitive `contains`; `needle` is lowercase.
pub fn contains_ci(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    if hay.is_ascii() && needle.is_ascii() {
        let (h, n) = (hay.as_bytes(), needle.as_bytes());
        return h.len() >= n.len() && h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n));
    }
    hay.to_lowercase().contains(needle)
}

struct Search {
    id: u64,
    needle: String,
    /// Matches of each page counted so far (from the first page on).
    counts: Vec<u32>,
    total: u64,
    /// The last match given: its rank is sent again once it can be known.
    shown: Option<Spot>,
}

struct MsgRef {
    offset: u64,
    len: u32,
    file: u16,
}

struct SortRef {
    at: i64,
    file: u16,
    seq: u32,
    offset: u64,
    len: u32,
}

const OPEN_FILE_CAP: usize = 16;

struct FilePool {
    paths: Vec<PathBuf>,
    /// Most-recently-used open file at the back; capped at `OPEN_FILE_CAP` so
    /// multi-file conversations never exhaust OS file descriptors.
    open: Vec<(u16, std::fs::File)>,
    /// File indices that failed to open or read, so they are logged once per
    /// window and never retried for every message.
    failed: Vec<bool>,
    /// Whether this window already logged a missing/unreadable file warning.
    warned: bool,
}

impl FilePool {
    fn new(paths: Vec<PathBuf>) -> Self {
        let len = paths.len();
        Self {
            paths,
            open: Vec::with_capacity(OPEN_FILE_CAP),
            failed: vec![false; len],
            warned: false,
        }
    }

    fn mark_failed(&mut self, idx: u16, kind: std::io::ErrorKind) {
        let Some(slot) = self.failed.get_mut(usize::from(idx)) else {
            return;
        };
        if *slot {
            return;
        }
        *slot = true;
        self.open.retain(|(id, _)| *id != idx);
        if !self.warned {
            self.warned = true;
            let failed_count = self.failed.iter().filter(|&&f| f).count();
            log::warn!(
                "conversation history: {failed_count} of {} files could not be read ({kind:?})",
                self.paths.len()
            );
        }
    }

    fn get_mut(&mut self, idx: u16) -> Option<&mut std::fs::File> {
        if self.failed.get(usize::from(idx)).copied().unwrap_or(true) {
            return None;
        }
        if let Some(pos) = self.open.iter().position(|(id, _)| *id == idx) {
            if pos + 1 != self.open.len() {
                let entry = self.open.remove(pos);
                self.open.push(entry);
            }
            return Some(&mut self.open.last_mut()?.1);
        }
        let path = self.paths.get(usize::from(idx))?;
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                self.mark_failed(idx, e.kind());
                return None;
            }
        };
        if self.open.len() >= OPEN_FILE_CAP {
            self.open.remove(0);
        }
        self.open.push((idx, file));
        Some(&mut self.open.last_mut()?.1)
    }
}

enum IndexedStorage {
    Single { file: std::fs::File, offsets: Vec<u64> },
    Multi { files: FilePool, messages: Vec<MsgRef> },
}

impl IndexedStorage {
    fn pages(&self) -> usize {
        match self {
            Self::Single { offsets, .. } => offsets.len(),
            Self::Multi { messages, .. } => messages.len().div_ceil(PAGE),
        }
    }

    fn page(&mut self, page: usize) -> Vec<Entry> {
        match self {
            Self::Single { file, offsets } => read_page(file, offsets, page).unwrap_or_default(),
            Self::Multi { files, messages } => read_page_multi(files, messages, page).unwrap_or_default(),
        }
    }
}

fn resolve_paths(source: &HistorySource) -> Vec<PathBuf> {
    match source {
        #[cfg(test)]
        HistorySource::File(path) => vec![path.clone()],
        HistorySource::Conversation { dir, peer, stem, shared } => {
            if *shared {
                names::all_candidate_files(dir, *peer, stem)
            } else {
                names::all_files_for_stem(dir, stem)
            }
        }
    }
}

fn dedup_hash(key: &str, h1: &RandomState, h2: &RandomState) -> u128 {
    let a = h1.hash_one(key);
    let b = h2.hash_one((key.len(), key));
    (u128::from(a) << 64) | u128::from(b)
}

/// First pass across several candidate files: stream each file one at a time
/// (closing each before opening the next), keeping only compact byte spans (no
/// full transcript text in RAM), deduplicate dated lines by minute, sender and
/// text while preserving within-file multiplicity (`store::dedup_key`; a line
/// without a date is always kept), and sort chronologically, a line without a
/// date after the message before it in the same file and a file without any
/// date at its modification time (`store::line_times`).
fn index_multi(paths: &[PathBuf], progress: &AtomicU32) -> std::io::Result<(FilePool, Vec<MsgRef>, u64)> {
    let total_est: u64 = paths
        .iter()
        .take(usize::from(u16::MAX))
        .map(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
        .fold(0u64, u64::saturating_add);

    let total_files = paths.len().min(usize::from(u16::MAX));
    let (h1, h2) = (RandomState::new(), RandomState::new());
    let mut max_occ: HashMap<u128, u32> = HashMap::new();
    let mut kept: Vec<SortRef> = Vec::new();
    let mut indexed_paths: Vec<PathBuf> = Vec::with_capacity(total_files);
    let mut indexed_bytes = 0u64;
    let mut failed_files = 0usize;
    let mut first_err_kind: Option<std::io::ErrorKind> = None;
    let mut last_err = None;
    let mut read_so_far = 0u64;
    let mut scanned = 0u64;
    let mut line = Vec::with_capacity(256);

    for path in paths.iter().take(total_files) {
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                failed_files += 1;
                first_err_kind.get_or_insert_with(|| e.kind());
                last_err = Some(e);
                continue;
            }
        };
        let file_idx = indexed_paths.len() as u16;
        let file_bytes = file.metadata().map(|m| m.len()).unwrap_or(0);
        indexed_paths.push(path.clone());
        indexed_bytes = indexed_bytes.saturating_add(file_bytes);

        let modified = modified_secs(path);
        let mut reader = BufReader::with_capacity(1 << 20, file);
        let mut at = 0u64;
        let mut pending_start = 0u64;
        let mut messages = Messages::default();
        // this file's messages: their date, duplicate key and byte span
        let mut spans: Vec<(Option<i64>, Option<u128>, u64, u32)> = Vec::new();

        let mut push_msg = |entry: Entry, start: u64, end: u64, progress_at: u64| {
            let hash = dedup_key(&entry).map(|key| dedup_hash(&key, &h1, &h2));
            let len = end.saturating_sub(start).min(u64::from(u32::MAX)) as u32;
            spans.push((entry_time(&entry), hash, start, len));
            scanned = scanned.saturating_add(1);
            if scanned.is_multiple_of(4096) && total_est > 0 {
                let done = read_so_far.saturating_add(progress_at);
                progress.store((done.saturating_mul(1000) / total_est).min(1000) as u32, Ordering::Relaxed);
            }
        };

        loop {
            let before = at;
            let read = match read_line(&mut reader, &mut line) {
                Ok(n) => n,
                Err(e) => {
                    failed_files += 1;
                    first_err_kind.get_or_insert_with(|| e.kind());
                    last_err = Some(e);
                    break;
                }
            };
            if read == 0 {
                break;
            }
            at += read;
            let had = messages.is_open();
            let flushed = if let Some(entry) = messages.line(&line) {
                push_msg(entry, pending_start, before, before);
                true
            } else {
                false
            };
            if (!had || flushed) && messages.is_open() {
                pending_start = before;
            }
        }
        if let Some(entry) = messages.finish() {
            push_msg(entry, pending_start, at, at);
        }
        let times: Vec<Option<i64>> = spans.iter().map(|s| s.0).collect();
        let mut file_occ: HashMap<u128, u32> = HashMap::new();
        for (seq, ((_, hash, offset, len), msg_at)) in spans.into_iter().zip(line_times(&times, modified)).enumerate() {
            if let Some(hash) = hash {
                let occ = file_occ.entry(hash).or_insert(0);
                *occ += 1;
                let seen = max_occ.entry(hash).or_insert(0);
                if *occ <= *seen {
                    continue;
                }
                *seen = *occ;
            }
            kept.push(SortRef {
                at: msg_at,
                file: file_idx,
                seq: seq.min(u32::MAX as usize) as u32,
                offset,
                len,
            });
        }
        read_so_far = read_so_far.saturating_add(at);
    }

    if indexed_paths.is_empty() {
        return Err(last_err.unwrap_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no transcript files")));
    }

    drop(max_occ);
    kept.sort_unstable_by(|a, b| a.at.cmp(&b.at).then_with(|| a.file.cmp(&b.file)).then_with(|| a.seq.cmp(&b.seq)));
    let out: Vec<MsgRef> = kept
        .into_iter()
        .map(|r| MsgRef {
            offset: r.offset,
            len: r.len,
            file: r.file,
        })
        .collect();
    let mut pool = FilePool::new(indexed_paths);
    if failed_files > 0 {
        pool.warned = true;
        let kind = first_err_kind.unwrap_or(std::io::ErrorKind::Other);
        log::warn!("conversation history: {failed_files} of {total_files} files could not be read ({kind:?})");
    }
    progress.store(1000, Ordering::Relaxed);
    Ok((pool, out, indexed_bytes))
}

fn read_page_multi(pool: &mut FilePool, refs: &[MsgRef], page: usize) -> std::io::Result<Vec<Entry>> {
    let Some(slice) = refs.get(page.saturating_mul(PAGE)..) else {
        return Ok(Vec::new());
    };
    let slice = &slice[..slice.len().min(PAGE)];
    let mut out = Vec::with_capacity(slice.len());
    let mut raw = Vec::with_capacity(512);
    for item in slice {
        let read_res = {
            let Some(file) = pool.get_mut(item.file) else {
                continue;
            };
            file.seek(SeekFrom::Start(item.offset)).and_then(|_| {
                raw.resize(item.len as usize, 0);
                file.read_exact(&mut raw)
            })
        };
        if let Err(e) = read_res {
            pool.mark_failed(item.file, e.kind());
            continue;
        }
        let mut messages = Messages::default();
        let trimmed = raw.strip_suffix(b"\n").unwrap_or(&raw);
        for line in trimmed.split(|&b| b == b'\n') {
            out.extend(messages.line(line));
        }
        out.extend(messages.finish());
    }
    Ok(out)
}

fn open_storage(source: &HistorySource, progress: &AtomicU32) -> std::io::Result<(IndexedStorage, Index)> {
    let paths = resolve_paths(source);
    match paths.as_slice() {
        [] => Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no transcript files")),
        [path] => {
            let (offsets, messages, bytes) = index(path, progress)?;
            let pages = offsets.len();
            // one handle for the pages; the writer appends through its own
            let file = std::fs::File::open(path)?;
            Ok((IndexedStorage::Single { file, offsets }, Index { pages, messages, bytes }))
        }
        _ => {
            let (files, refs, bytes) = index_multi(&paths, progress)?;
            let messages = refs.len() as u64;
            let storage = IndexedStorage::Multi { files, messages: refs };
            let pages = storage.pages();
            Ok((storage, Index { pages, messages, bytes }))
        }
    }
}

fn run(source: &HistorySource, cmds: Receiver<Cmd>, replies: Sender<Reply>, progress: &AtomicU32) {
    let (mut storage, index) = match open_storage(source, progress) {
        Ok(v) => v,
        Err(e) => {
            log::info!("conversation history: transcript not read ({:?})", e.kind());
            let _ = replies.send(Reply::Indexed(None));
            return;
        }
    };
    let pages = index.pages;
    if replies.send(Reply::Indexed(Some(index))).is_err() {
        return;
    }
    let page_hits = |entries: Vec<Entry>, needle: &str| -> Vec<usize> {
        entries.iter().enumerate().filter(|(_, e)| hit(e, needle)).map(|(i, _)| i).collect()
    };
    let mut search: Option<Search> = None;
    loop {
        // while a search is counting, commands are served between two pages
        let counting = search.as_ref().is_some_and(|s| s.counts.len() < pages);
        let cmd = if counting {
            match cmds.try_recv() {
                Ok(c) => Some(c),
                Err(crossbeam_channel::TryRecvError::Empty) => None,
                Err(crossbeam_channel::TryRecvError::Disconnected) => return,
            }
        } else {
            match cmds.recv() {
                Ok(c) => Some(c),
                Err(_) => return,
            }
        };
        let sent = match cmd {
            Some(Cmd::Page(page)) => replies.send(Reply::Page {
                page,
                entries: storage.page(page),
            }),
            Some(Cmd::Search { id, needle }) => {
                search = (!needle.is_empty()).then(|| Search {
                    id,
                    needle,
                    counts: Vec::new(),
                    total: 0,
                    shown: None,
                });
                replies.send(Reply::Counted {
                    id,
                    total: 0,
                    done: search.is_none() || pages == 0,
                })
            }
            Some(Cmd::Step { id, from, forward }) => {
                let Some(s) = search.as_mut().filter(|s| s.id == id) else {
                    continue;
                };
                let mut found = None;
                // from the spot given, else from the end of the file
                let mut walk = (pages > 0).then(|| from.map_or(pages - 1, |f| f.page.min(pages - 1)));
                let mut first = true;
                while let Some(page) = walk {
                    // a page already counted as empty is not read again
                    let known_empty = s.counts.get(page) == Some(&0);
                    let hits = if known_empty {
                        Vec::new()
                    } else {
                        page_hits(storage.page(page), &s.needle)
                    };
                    let at = match (from.filter(|_| first), forward) {
                        (Some(f), true) => hits.iter().copied().find(|&m| m > f.message),
                        (Some(f), false) => hits.iter().rev().copied().find(|&m| m < f.message),
                        (None, true) => hits.first().copied(),
                        (None, false) => hits.last().copied(),
                    };
                    first = false;
                    if let Some(message) = at {
                        let before: u64 = hits.iter().filter(|&&m| m < message).count() as u64;
                        let rank = (s.counts.len() >= page).then(|| s.counts[..page].iter().map(|&c| u64::from(c)).sum::<u64>() + before);
                        found = Some((Spot { page, message }, rank));
                        break;
                    }
                    // no wrapping around: the ends of the file stop the walk
                    walk = if forward {
                        Some(page + 1).filter(|&p| p < pages)
                    } else {
                        page.checked_sub(1)
                    };
                }
                s.shown = found.map(|(spot, _)| spot).or(s.shown);
                replies.send(Reply::Found {
                    id,
                    spot: found.map(|(s, _)| s),
                    rank: found.and_then(|(_, r)| r),
                })
            }
            None => {
                let Some(s) = search.as_mut() else {
                    continue;
                };
                let page = s.counts.len();
                let n = page_hits(storage.page(page), &s.needle).len() as u32;
                s.counts.push(n);
                s.total += u64::from(n);
                let done = s.counts.len() >= pages;
                // not a message per page: every 64 pages and at the end
                let mut sent = Ok(());
                if done || s.counts.len().is_multiple_of(64) {
                    sent = replies.send(Reply::Counted {
                        id: s.id,
                        total: s.total,
                        done,
                    });
                }
                // the match shown was found before the pages ahead of it
                // were counted: its rank is known now
                if let Some(spot) = s.shown.filter(|_| done) {
                    let before = page_hits(storage.page(spot.page), &s.needle)
                        .iter()
                        .filter(|&&m| m < spot.message)
                        .count() as u64;
                    let rank = s.counts[..spot.page.min(s.counts.len())].iter().map(|&c| u64::from(c)).sum::<u64>() + before;
                    sent = sent.and(replies.send(Reply::Found {
                        id: s.id,
                        spot: Some(spot),
                        rank: Some(rank),
                    }));
                }
                sent
            }
        };
        if sent.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(120);

    /// A folder of the temp folder, removed with its files when the test
    /// ends, also when it fails.
    struct Temp(PathBuf);

    impl Temp {
        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp() -> Temp {
        let dir = std::env::temp_dir().join(format!("aurora-transcript-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Temp(dir)
    }

    /// `messages` messages, every seventh on two lines, "needle" in one of
    /// every thousand.
    fn generate(path: &Path, messages: u64) -> u64 {
        let mut w = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(path).unwrap());
        let mut bytes = 0u64;
        for i in 0..messages {
            let mut line = format!("[2026/10/12 14:05]  Jane Doe: message {i} of a long conversation about nothing");
            if i % 1000 == 500 {
                line.push_str(" with a Needle in it");
            }
            if i % 7 == 3 {
                line.push_str("\r\n and its second line");
            }
            line.push_str("\r\n");
            bytes += line.len() as u64;
            w.write_all(line.as_bytes()).unwrap();
        }
        w.flush().unwrap();
        bytes
    }

    fn indexed(t: &Transcript) -> Index {
        match t.replies.recv_timeout(WAIT).unwrap() {
            Reply::Indexed(Some(i)) => i,
            other => panic!("{other:?}"),
        }
    }

    fn page(t: &Transcript, n: usize) -> Vec<Entry> {
        t.page(n);
        loop {
            match t.replies.recv_timeout(WAIT).unwrap() {
                Reply::Page { page, entries } if page == n => return entries,
                _ => {}
            }
        }
    }

    fn counted(t: &Transcript, search: u64) -> u64 {
        loop {
            match t.replies.recv_timeout(WAIT).unwrap() {
                Reply::Counted { id, total, done: true } if id == search => return total,
                _ => {}
            }
        }
    }

    fn found(t: &Transcript, search: u64) -> (Option<Spot>, Option<u64>) {
        loop {
            match t.replies.recv_timeout(WAIT).unwrap() {
                Reply::Found { id, spot, rank } if id == search => return (spot, rank),
                _ => {}
            }
        }
    }

    #[test]
    fn pages_of_a_hundred_messages() {
        let dir = temp();
        let path = dir.join("Jane Doe.txt");
        let bytes = generate(&path, 12_345);
        let t = Transcript::open(path).unwrap();
        let i = indexed(&t);
        assert_eq!(
            i,
            Index {
                pages: 124,
                messages: 12_345,
                bytes
            }
        );
        assert_eq!(t.progress(), 1.0);
        let last = page(&t, 123);
        assert_eq!(last.len(), 45);
        assert!(last.last().unwrap().text.starts_with("message 12344 "));
        let first = page(&t, 0);
        assert_eq!(first.len(), PAGE);
        assert!(first[0].text.starts_with("message 0 "));
        // a two-line message is whole
        assert!(first[3].text.ends_with("\nand its second line"));
        assert!(first[99].text.starts_with("message 99 "));
        assert!(page(&t, 50)[0].text.starts_with("message 5000 "));
        assert!(page(&t, 124).is_empty());
    }

    #[test]
    fn a_search_counts_and_walks_the_whole_file() {
        let dir = temp();
        let path = dir.join("Jane Doe.txt");
        generate(&path, 12_345);
        let t = Transcript::open(path).unwrap();
        indexed(&t);
        t.search(1, "needle".into());
        assert_eq!(counted(&t, 1), 12);
        // from the end, backward: the last match, the twelfth
        t.step(1, None, false);
        let (spot, rank) = found(&t, 1);
        assert_eq!(spot, Some(Spot { page: 115, message: 0 }));
        assert_eq!(rank, Some(11));
        // the one before it
        t.step(1, spot, false);
        assert_eq!(found(&t, 1), (Some(Spot { page: 105, message: 0 }), Some(10)));
        // forward again, then past the last one: nothing, no wrapping
        t.step(1, Some(Spot { page: 105, message: 0 }), true);
        assert_eq!(found(&t, 1).0, spot);
        t.step(1, spot, true);
        assert_eq!(found(&t, 1), (None, None));
        // a sender's name matches too, in every message
        t.search(2, "jane".into());
        assert_eq!(counted(&t, 2), 12_345);
        // an answer to an older search is not given
        t.step(1, None, false);
        t.search(3, "nothing here".into());
        assert_eq!(counted(&t, 3), 0);
        t.step(3, None, false);
        assert_eq!(found(&t, 3), (None, None));
    }

    #[test]
    fn odd_files() {
        let dir = temp();
        // missing
        let t = Transcript::open(dir.join("missing.txt")).unwrap();
        assert!(matches!(t.replies.recv_timeout(WAIT).unwrap(), Reply::Indexed(None)));
        // empty
        let path = dir.join("empty.txt");
        std::fs::write(&path, b"").unwrap();
        let t = Transcript::open(path).unwrap();
        assert_eq!(indexed(&t).pages, 0);
        assert!(page(&t, 0).is_empty());
        t.search(1, "x".into());
        assert_eq!(counted(&t, 1), 0);
        // one line far longer than the bound, without an end of line, then
        // nothing: one message, cut
        let path = dir.join("long.txt");
        let mut bytes = b"[2026/10/12 14:05]  Jane Doe: ".to_vec();
        bytes.extend(std::iter::repeat_n(b'x', 5_000_000));
        std::fs::write(&path, &bytes).unwrap();
        let t = Transcript::open(path).unwrap();
        assert_eq!(indexed(&t).messages, 1);
        let p = page(&t, 0);
        assert_eq!(p.len(), 1);
        assert!(p[0].text.len() <= MAX_LINE);
    }

    #[test]
    fn combines_split_files_for_pages_and_search() {
        let dir = temp();
        let jane = Uuid::from_u128(0x42);
        let session = Uuid::from_u128(0x99);

        // Missing source yields Indexed(None) without main-thread I/O
        let empty = Transcript::open_source(HistorySource::Conversation {
            dir: dir.0.clone(),
            peer: Some(jane),
            stem: "Jane Doe".into(),
            shared: true,
        })
        .unwrap();
        assert!(matches!(empty.replies.recv_timeout(WAIT).unwrap(), Reply::Indexed(None)));

        // Spread > PAGE messages across dated Legacy, dated username, undated, and conversation.log custom files
        let mut sept_legacy = String::from("[2026/09/10 10:00]  Jane Doe: from sept\r\n");
        for i in 0..80 {
            let extra = if i % 2 == 0 { "\r\n continuation line" } else { "" };
            sept_legacy.push_str(&format!("[2026/09/11 10:{i:02}]  Jane Doe: bulk sept {i}{extra}\r\n"));
        }
        // Duplicate of a line that also appears in oct_user, plus a twice-repeated line
        sept_legacy.push_str("[2026/09/15 12:00]  Jane Doe: shared milestone\r\n");
        sept_legacy.push_str("[2026/09/15 12:01]  Jane Doe: echo\r\n");
        sept_legacy.push_str("[2026/09/15 12:01]  Jane Doe: echo\r\n");
        std::fs::write(dir.join("Jane Doe-2026-09.txt"), &sept_legacy).unwrap();

        let mut oct_user = String::from(
            "\u{FEFF}[2026/09/15 12:00:30]  Jane Doe: shared milestone\n\
             [2026/09/15 12:01]  Jane Doe: echo\n",
        );
        for i in 0..45 {
            oct_user.push_str(&format!("[2026/10/01 11:{i:02}]  Jane Doe: bulk oct {i}\n"));
        }
        std::fs::write(dir.join("jane_doe-2026-10.txt"), &oct_user).unwrap();

        // Undated line in conversation.log custom file stays anchored after its dated predecessor
        std::fs::write(
            dir.join("conversation.log"),
            format!("[1700000500] 0 0 0 Jane%20Doe| {jane} {session} jane.custom|\n"),
        )
        .unwrap();
        std::fs::write(
            dir.join("jane_custom.txt"),
            "[2026/09/10 10:05]  Jane Doe: needle custom dated\n[10:06]  Jane Doe: custom undated tail\n",
        )
        .unwrap();
        // Undated Legacy file has a latest message
        std::fs::write(dir.join("Jane Doe.txt"), "[2026/10/02 09:00]  Jane Doe: needle latest\n").unwrap();

        let source = HistorySource::Conversation {
            dir: dir.0.clone(),
            peer: Some(jane),
            stem: "Jane Doe".into(),
            shared: true,
        };
        let t = Transcript::open_source(source).unwrap();
        let idx = indexed(&t);
        // 1 (10:00) + 80 (sept) + 1 (shared milestone) + 2 (echo) + 45 (oct) + 2 (custom) + 1 (latest) = 132 messages
        assert_eq!(idx.messages, 132);
        assert_eq!(idx.pages, 2);

        let p0 = page(&t, 0);
        let p1 = page(&t, 1);
        assert_eq!(p0.len(), PAGE);
        assert_eq!(p1.len(), 32);
        assert_eq!(p0[0].text, "from sept");
        assert_eq!(p0[1].text, "needle custom dated");
        assert_eq!(p0[2].text, "custom undated tail");
        assert_eq!(p0[3].text, "bulk sept 0\ncontinuation line");
        assert_eq!(p1.last().unwrap().text, "needle latest");

        // Multi-file search & stepping across pages
        t.search(1, "needle".into());
        assert_eq!(counted(&t, 1), 2);
        t.step(1, None, false);
        assert_eq!(found(&t, 1), (Some(Spot { page: 1, message: 31 }), Some(1)));
        t.step(1, Some(Spot { page: 1, message: 31 }), false);
        assert_eq!(found(&t, 1), (Some(Spot { page: 0, message: 1 }), Some(0)));
        assert_eq!(
            p1.iter().filter(|e| e.text == "shared milestone").count() + p0.iter().filter(|e| e.text == "shared milestone").count(),
            1
        );
        assert_eq!(
            p1.iter().filter(|e| e.text == "echo").count() + p0.iter().filter(|e| e.text == "echo").count(),
            2
        );
    }

    #[test]
    fn indexes_more_files_than_open_handle_cap_and_reads_all_pages() {
        let dir = temp();
        let count = 120usize;
        assert!(count > OPEN_FILE_CAP);
        assert!(count > PAGE);

        // Spread 120 messages across 120 daily files (one file per day).
        for i in 0..count {
            let month = 1 + (i / 28);
            let day = 1 + (i % 28);
            let name = format!("chat-2025-{month:02}-{day:02}.txt");
            let line = format!("[2025/{month:02}/{day:02} 12:00]  Jane Doe: day {i:03}\n");
            std::fs::write(dir.join(&name), line).unwrap();
        }

        // Indexing directly also skips any unopenable path instead of failing the rest.
        let mut paths: Vec<PathBuf> = (0..count)
            .map(|i| {
                let month = 1 + (i / 28);
                let day = 1 + (i % 28);
                dir.join(&format!("chat-2025-{month:02}-{day:02}.txt"))
            })
            .collect();
        paths.insert(5, dir.join("missing-2025-01-99.txt"));
        let progress = AtomicU32::new(0);
        let (mut pool, refs, _) = index_multi(&paths, &progress).unwrap();
        assert_eq!(refs.len(), count);
        let p0_direct = read_page_multi(&mut pool, &refs, 0).unwrap();
        assert_eq!(p0_direct.len(), PAGE);
        assert!(pool.open.len() <= OPEN_FILE_CAP);

        // End-to-end through `Transcript` across >100 files and multiple pages + search.
        let t = Transcript::open_source(HistorySource::Conversation {
            dir: dir.0.clone(),
            peer: None,
            stem: "chat".into(),
            shared: true,
        })
        .unwrap();
        let idx = indexed(&t);
        assert_eq!(idx.messages, count as u64);
        assert_eq!(idx.pages, 2);

        let p0 = page(&t, 0);
        let p1 = page(&t, 1);
        let p0_again = page(&t, 0);
        assert_eq!(p0.len(), PAGE);
        assert_eq!(p1.len(), count - PAGE);
        assert_eq!(p0_again.len(), PAGE);
        assert_eq!(p0[0].text, "day 000");
        assert_eq!(p0.last().unwrap().text, "day 099");
        assert_eq!(p1[0].text, "day 100");
        assert_eq!(p1.last().unwrap().text, "day 119");

        t.search(1, "day ".into());
        assert_eq!(counted(&t, 1), count as u64);

        // Removing an already-indexed file mid-window marks that file index failed once
        // and keeps returning the remaining messages on repeat page reads.
        paths.remove(5);
        let (mut clean_pool, clean_refs, _) = index_multi(&paths, &progress).unwrap();
        assert!(!clean_pool.warned);
        assert!(clean_pool.failed.iter().all(|&f| !f));
        std::fs::remove_file(&paths[0]).unwrap();
        let p0_after_delete = read_page_multi(&mut clean_pool, &clean_refs, 0).unwrap();
        assert_eq!(p0_after_delete.len(), PAGE - 1);
        assert_eq!(p0_after_delete[0].text, "day 001");
        assert!(clean_pool.failed[0]);
        assert!(clean_pool.warned);
        assert!(clean_pool.get_mut(0).is_none());
        let p0_repeat = read_page_multi(&mut clean_pool, &clean_refs, 0).unwrap();
        assert_eq!(p0_repeat.len(), PAGE - 1);
    }

    #[test]
    fn search_text_matching() {
        assert!(contains_ci("Hello World", "lo wo"));
        assert!(contains_ci("ÇA VA", "ça"));
        assert!(!contains_ci("abc", ""));
        assert!(!contains_ci("ab", "abc"));
    }

    /// A transcript of several hundred megabytes: indexed, paged and
    /// searched without holding it in memory. Run by hand (it writes 300 MB
    /// in the temp folder): `cargo test -p aurora-viewer huge_transcript --
    /// --ignored --nocapture`.
    #[test]
    #[ignore = "writes a 300 MB file"]
    fn huge_transcript() {
        let dir = temp();
        let path = dir.join("huge.txt");
        let started = std::time::Instant::now();
        let bytes = generate(&path, 3_700_000);
        println!("written: {} MB in {:?}", bytes >> 20, started.elapsed());
        assert!(bytes > 300 << 20);
        let started = std::time::Instant::now();
        let t = Transcript::open(path).unwrap();
        let i = indexed(&t);
        println!("indexed: {} pages, {} messages in {:?}", i.pages, i.messages, started.elapsed());
        assert_eq!(i.messages, 3_700_000);
        let started = std::time::Instant::now();
        let last = page(&t, i.pages - 1);
        println!("last page: {:?}", started.elapsed());
        assert!(last.last().unwrap().text.starts_with("message 3699999 "));
        let started = std::time::Instant::now();
        t.search(1, "needle".into());
        // a page is served while the search counts
        let middle = page(&t, i.pages / 2);
        println!("a page during the search: {:?}", started.elapsed());
        assert_eq!(middle.len(), PAGE);
        assert_eq!(counted(&t, 1), 3_700);
        println!("search of the whole file: {:?}", started.elapsed());
        t.step(1, None, false);
        assert_eq!(found(&t, 1).1, Some(3_699));
    }

    #[test]
    fn time_only_lines_are_never_merged_and_their_files_follow_their_time() {
        let dir = temp();
        let set_modified = |name: &str, secs: u64| {
            let file = std::fs::File::options().write(true).open(dir.join(name)).unwrap();
            file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(secs)).unwrap();
        };
        // two days' files with times only (LogTimestampDate off): the same
        // short line at the same minute on both days stays twice
        std::fs::write(
            dir.join("Jane Doe.txt"),
            "[14:05]  Jane Doe: same\n[14:06]  Jane Doe: from the newer day\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("jane_doe.txt"),
            "[14:05]  Jane Doe: same\n[14:06]  Jane Doe: from the older day\n",
        )
        .unwrap();
        set_modified("jane_doe.txt", 1_790_000_000);
        set_modified("Jane Doe.txt", 1_793_000_000);
        // dated lines between them, merged once across two files
        std::fs::write(dir.join("Jane Doe-2026-10.txt"), "[2026/10/12 14:05]  Jane Doe: dated\n").unwrap();
        std::fs::write(dir.join("jane_doe-2026-10.txt"), "[2026/10/12 14:05]  Jane Doe: dated\n").unwrap();
        let t = Transcript::open_source(HistorySource::Conversation {
            dir: dir.0.clone(),
            peer: None,
            stem: "Jane Doe".into(),
            shared: true,
        })
        .unwrap();
        assert_eq!(indexed(&t).messages, 5);
        let texts: Vec<String> = page(&t, 0).into_iter().map(|e| e.text).collect();
        assert_eq!(texts, ["same", "from the older day", "dated", "same", "from the newer day"]);
    }
}
