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
use super::store::{MAX_LINE, Messages};
use crossbeam_channel::{Receiver, Sender};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// CONVERSATION_HISTORY_PAGE_SIZE.
pub const PAGE: usize = 100;

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
    pub path: PathBuf,
    cmds: Sender<Cmd>,
    pub replies: Receiver<Reply>,
    /// Part of the file read by the first pass, in thousandths.
    progress: Arc<AtomicU32>,
}

impl Transcript {
    /// Start reading a file. None when the thread cannot start.
    pub fn open(path: PathBuf) -> Option<Transcript> {
        let (cmds, rx) = crossbeam_channel::unbounded();
        let (tx, replies) = crossbeam_channel::unbounded();
        let progress = Arc::new(AtomicU32::new(0));
        let (p, shared) = (path.clone(), progress.clone());
        std::thread::Builder::new()
            .name("aurora-transcript".into())
            .spawn(move || run(&p, rx, tx, &shared))
            .ok()?;
        Some(Transcript {
            path,
            cmds,
            replies,
            progress,
        })
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

fn run(path: &Path, cmds: Receiver<Cmd>, replies: Sender<Reply>, progress: &AtomicU32) {
    let (offsets, messages, bytes) = match index(path, progress) {
        Ok(v) => v,
        Err(e) => {
            log::info!("conversation history: transcript not read ({e})");
            let _ = replies.send(Reply::Indexed(None));
            return;
        }
    };
    let pages = offsets.len();
    if replies.send(Reply::Indexed(Some(Index { pages, messages, bytes }))).is_err() {
        return;
    }
    // one handle for the pages; the writer appends through its own
    let Ok(mut file) = std::fs::File::open(path) else {
        return;
    };
    let mut page_of = |page: usize| read_page(&mut file, &offsets, page).unwrap_or_default();
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
                entries: page_of(page),
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
                        page_hits(page_of(page), &s.needle)
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
                let n = page_hits(page_of(page), &s.needle).len() as u32;
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
                    let before = page_hits(page_of(spot.page), &s.needle)
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
}
