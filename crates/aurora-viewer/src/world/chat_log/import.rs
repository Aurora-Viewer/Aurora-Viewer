//! Bring a Legacy Viewer's transcripts into Aurora, one way: its files are
//! only read. The text of a line and the name of a file are the same on
//! both sides (`format`, `names`), so a line is kept as written; what differs
//! is the folder, `<logs>/<first_last>[.<grid label>]` there
//! (LLDir::setPerAccountChatLogsDir, indra/llfilesystem/lldir.cpp, originally
//! LGPL 2.1), found by `legacy_profile`, one folder per account id and grid
//! here (`store::folder`).
//!
//! A conversation both viewers have is merged by time, with what the
//! Aurora file already holds left out, so importing again adds nothing.
//! Merging rewrites the file: one file at a time on the transcript thread
//! (`store::Job::Merge`), so that the lines of a live conversation wait for
//! that file only, then a rename, so that a failure leaves it as it was.

use super::{format, names, store};
pub use crate::legacy_profile::Account;
use crate::legacy_profile::account_folders;
use crossbeam_channel::{Receiver, Sender};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The per-account settings file, where a chosen log folder is kept.
const ACCOUNT_SETTINGS: &str = "settings_per_account.xml";

/// A folder of transcripts, as shown before importing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub dir: PathBuf,
    pub files: usize,
    pub bytes: u64,
    /// The folder Aurora writes to, through a link: nothing to import.
    pub same: bool,
}

/// Where a viewer keeps the logs of one account: the InstantMessageLogPath
/// of its settings, else the application folder, as LLStartUp does when the
/// setting is empty or names a folder that does not exist.
fn log_root(app: &Path, account_folder: &Path) -> PathBuf {
    let chosen = std::fs::read(account_folder.join(ACCOUNT_SETTINGS))
        .ok()
        .and_then(|bytes| aurora_llsd::from_xml(&bytes).ok())
        .map(|settings| settings.get("InstantMessageLogPath").get("Value").as_str().trim().to_owned())
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    chosen.filter(|path| path.is_dir()).unwrap_or_else(|| app.to_path_buf())
}

/// The account's transcript folders under the viewers' folders, a folder
/// shared by two viewers once.
pub fn find(apps: &[PathBuf], account: &Account, target: &Path) -> Vec<Found> {
    let mut seen = Vec::new();
    let mut found = Vec::new();
    for folder in account_folders(apps, account) {
        let (Some(app), Some(name)) = (folder.parent(), folder.file_name()) else {
            continue;
        };
        let dir = log_root(app, &folder).join(name);
        let real = dir.canonicalize().unwrap_or_else(|_| dir.clone());
        if seen.contains(&real) {
            continue;
        }
        seen.push(real);
        let here = inspect(&dir, target);
        if here.files > 0 {
            found.push(here);
        }
    }
    found
}

/// What a folder holds. A folder given by hand may be the viewer's own, or
/// the one holding the account's: the account's is looked for in it.
pub fn inspect_chosen(dir: &Path, account: &Account, target: &Path) -> Found {
    let here = inspect(dir, target);
    if here.files > 0 {
        return here;
    }
    let inner = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .find(|e| e.file_name().to_str().is_some_and(|n| account.owns(n)));
    match inner {
        Some(entry) => inspect(&entry.path(), target),
        None => here,
    }
}

fn inspect(dir: &Path, target: &Path) -> Found {
    let files = transcripts(dir);
    let bytes = files.iter().filter_map(|f| f.metadata().ok()).map(|m| m.len()).sum();
    Found {
        dir: dir.to_path_buf(),
        files: files.len(),
        bytes,
        same: same_path(dir, target),
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

/// The transcripts of a folder, sorted. The folder is also the account's
/// settings folder on the viewer's side, which has text files that are not
/// transcripts (search_history.txt, teleport_history.txt…).
pub fn transcripts(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = text_files(dir).filter(|p| kind(p) == Kind::Transcript).collect();
    files.sort();
    files
}

/// The folder's files that may be conversations but carry no timestamp.
fn left_out(dir: &Path) -> usize {
    text_files(dir).filter(|p| kind(p) == Kind::NoStamp).count()
}

fn text_files(dir: &Path) -> impl Iterator<Item = PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(names::EXTENSION)))
}

/// What a text file of a Legacy Viewer's folder is to the import.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Transcript,
    /// Text without a timestamp, or a file that cannot be read: told in the
    /// report, as a conversation logged with LogTimestamp off looks like this.
    NoStamp,
    Other,
}

/// LLLogChat::isTranscriptFileFound asks for a timestamp on the first line,
/// to offer a history panel; here it would drop a whole conversation that
/// began without one, so any line of the head will do. The nearby chat's
/// file always counts.
fn kind(path: &Path) -> Kind {
    use std::io::Read;
    if !path.is_file() {
        return Kind::Other;
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if undated(stem) == names::NEARBY {
        return Kind::Transcript;
    }
    let mut head = Vec::with_capacity(64);
    if std::fs::File::open(path)
        .and_then(|f| f.take(store::LOG_RECALL_SIZE).read_to_end(&mut head))
        .is_err()
    {
        // unreadable now (locked, denied): its merge fails and is counted
        // as failed, which says to import again, not as left out
        return Kind::Transcript;
    }
    let head = head.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&head);
    let head = String::from_utf8_lossy(head);
    if head.lines().any(format::starts_with_stamp) {
        return Kind::Transcript;
    }
    // the settings files in that folder are LLSD
    match head.trim_start().chars().next() {
        None | Some('<' | '{') => Kind::Other,
        Some(_) => Kind::NoStamp,
    }
}

/// A file's name without the date of LogFileNamewithDate, which only says
/// how the viewer cut the conversation: `chat-2026-10-12`, `Jane Doe-2026-10`.
pub fn undated(stem: &str) -> &str {
    let date = |s: &str, pattern: &str| {
        s.bytes()
            .zip(pattern.bytes())
            .all(|(b, p)| if p == b'0' { b.is_ascii_digit() } else { b == p })
    };
    let cut = |pattern: &str| {
        stem.len()
            .checked_sub(pattern.len())
            .filter(|&i| stem.is_char_boundary(i) && date(&stem[i..], pattern))
    };
    if let Some(i) = cut("-0000-00-00")
        && &stem[..i] == names::NEARBY
    {
        return names::NEARBY;
    }
    match cut("-0000-00") {
        Some(i) if i > 0 => &stem[..i],
        _ => stem,
    }
}

/// The file in Aurora's folder that receives a Legacy file. With dated names
/// off a conversation is one file, as it is here, or the months Aurora
/// does not read back would stay out of its history.
fn target_of(dir: &Path, source: &Path, dated: bool, spelled: &Spellings) -> Option<PathBuf> {
    let stem = source.file_stem()?.to_str()?;
    let stem = once_group(if dated { stem } else { undated(stem) });
    names::file_path(dir, &resident_name(&stem, spelled), None)
}

/// The files' names with their capitals, by their name in lower case.
type Spellings = HashMap<String, String>;

/// What Aurora's folder and the sources say of the capitals of a name that
/// lost them. Aurora's come first: that file is the one it reads.
fn spellings(dir: &Path, sources: &[PathBuf]) -> Spellings {
    let mut spelled = Spellings::new();
    let mut add = |file: &Path| {
        let Some(stem) = file.file_stem().and_then(|s| s.to_str()) else {
            return;
        };
        let base = undated(stem);
        if base != base.to_lowercase() && !base.ends_with(names::GROUP_SUFFIX) {
            spelled.entry(base.to_lowercase()).or_insert_with(|| base.to_owned());
        }
    };
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        add(&entry.path());
    }
    sources.iter().for_each(|source| add(source));
    spelled
}

/// `jane_doe` as Aurora names that resident's file, `Jane Doe`: a Legacy Viewer
/// with UseLegacyIMLogNames off, or without it, names it from
/// LLCacheName::buildUsername, then cleanFileName turns its dot into '_'.
/// That leaves the name in lower case, and Aurora's file of `JaneDoe` or
/// `Jane McDoe` keeps its capitals: taken from `spelled`, or each word's
/// first one comes back when nothing says more.
fn resident_name(stem: &str, spelled: &Spellings) -> String {
    let base = undated(stem);
    if base != base.to_lowercase() || base == names::NEARBY {
        return stem.to_owned();
    }
    let tail = &stem[base.len()..];
    if let Some(known) = spelled.get(&base.replacen('_', " ", 1)) {
        return format!("{known}{tail}");
    }
    let word = |w: &str| !w.is_empty() && w.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    let capital = |w: &str| w[..1].to_ascii_uppercase() + &w[1..];
    match base.split_once('_') {
        Some((first, last)) if word(first) && word(last) => format!("{} {}{tail}", capital(first), capital(last)),
        _ => stem.to_owned(),
    }
}

/// A group's file with one " (group)": Legacy Viewers leave many as
/// `Club (group) (group)`, beside or instead of `Club (group)`, and Aurora
/// reads only the second. In a dated name the suffix is before the date.
fn once_group(stem: &str) -> String {
    let twice = format!("{0}{0}", names::GROUP_SUFFIX);
    match stem.rfind(&twice) {
        Some(mut i) => {
            let tail = &stem[i + twice.len()..];
            while stem[..i].ends_with(names::GROUP_SUFFIX) {
                i -= names::GROUP_SUFFIX.len();
            }
            format!("{}{}{tail}", &stem[..i], names::GROUP_SUFFIX)
        }
        None => stem.to_owned(),
    }
}

/// A message of a transcript: its lines in the file, what tells it from
/// another and the moment used to place it.
struct Message {
    range: Range<usize>,
    key: String,
    at: i64,
    /// The stamp's minute and who said what, for a dated message.
    said: Option<(i64, String)>,
}

/// The messages of a file: a line starting with a space or an empty one
/// continues the previous message (`store::Messages`). The bytes stay as
/// they are; only the key decodes them.
fn messages(bytes: &[u8]) -> Vec<Message> {
    let start = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) { 3 } else { 0 };
    let mut ranges: Vec<Range<usize>> = Vec::new();
    let mut pos = start;
    for line in bytes[start..].split(|&b| b == b'\n') {
        let end = pos + line.len();
        let more = matches!(line.first(), None | Some(b' ' | b'\r'));
        match ranges.last_mut() {
            Some(r) if more => r.end = end,
            None if line.iter().all(|&b| b == b'\r') => {}
            _ => ranges.push(pos..end),
        }
        pos = end + 1;
    }
    let mut at = i64::MIN;
    let mut said = None;
    ranges
        .into_iter()
        .map(|mut range| {
            while range.end > range.start && matches!(bytes[range.end - 1], b'\n' | b'\r') {
                range.end -= 1;
            }
            let message = &bytes[range.clone()];
            let key = match store::parse(message, false).into_iter().next() {
                Some(entry) => {
                    // a line without a date is placed after the one before it
                    let minute = entry.time.map(|t| unix(t) / 60);
                    at = minute.unwrap_or(at);
                    said = minute.map(|minute| (minute, format!("{}\0{}", entry.from, entry.text)));
                    // the minute, whether the stamp is 12 or 24 hours and has
                    // seconds; a stamp without a date as it is
                    match minute {
                        Some(minute) => format!("{minute}\0{}\0{}", entry.from, entry.text),
                        None => format!("{}\0{}\0{}", entry.stamp, entry.from, entry.text),
                    }
                }
                None => {
                    said = None;
                    String::from_utf8_lossy(message).into_owned()
                }
            };
            Message {
                range,
                key,
                at,
                said: said.take(),
            }
        })
        .collect()
}

fn unix(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn put(out: &mut Vec<u8>, bytes: &[u8], message: &Message) {
    for line in bytes[message.range.clone()].split(|&b| b == b'\n') {
        out.extend_from_slice(line.strip_suffix(b"\r").unwrap_or(line));
        out.extend_from_slice(store::EOL.as_bytes());
    }
}

/// Which of `theirs` are group chat history written again. Opening a group
/// chat fetches the recent messages from the server, and
/// LLIMSession::addMessagesFromServerHistory (indra/newview/llimview.cpp)
/// logs them with the moment of the fetch, so the same lines come back under
/// a new stamp at each login. A minute is taken for that only when it has at
/// least two messages and every one was said before it: a line said again on
/// its own, or a minute with anything new, stays. One replayed line alone
/// therefore stays too.
fn replayed(mine: &[Message], other: &[Message]) -> Vec<bool> {
    let mut first: HashMap<&str, i64> = HashMap::with_capacity(mine.len() + other.len());
    for (minute, said) in mine.iter().chain(other).filter_map(|m| m.said.as_ref()) {
        first
            .entry(said.as_str())
            .and_modify(|at| *at = (*at).min(*minute))
            .or_insert(*minute);
    }
    // messages of the minute, and whether all were said before it
    let mut minutes: HashMap<i64, (u32, bool)> = HashMap::new();
    for (minute, said) in other.iter().filter_map(|m| m.said.as_ref()) {
        let entry = minutes.entry(*minute).or_insert((0, true));
        entry.0 += 1;
        entry.1 &= first.get(said.as_str()).is_some_and(|at| at < minute);
    }
    other
        .iter()
        .map(|m| {
            let minute = m.said.as_ref().and_then(|(minute, _)| minutes.get(minute));
            minute.is_some_and(|&(count, before)| count >= 2 && before)
        })
        .collect()
}

/// The two files as one, by time, each keeping its own order, and the number
/// of messages added; None when `theirs` adds nothing. A message counts as
/// often as it appears: "ok" sent twice in a minute stays twice, and is not
/// added again once it is there. In a group's file, history written again
/// is left out (`replayed`).
fn merged(ours: &[u8], theirs: &[u8], group: bool) -> Option<(Vec<u8>, u64)> {
    let mine = messages(ours);
    let other = messages(theirs);
    let replay = if group { replayed(&mine, &other) } else { vec![false; other.len()] };
    let mut have: HashMap<&str, u32> = HashMap::with_capacity(mine.len());
    for m in &mine {
        *have.entry(m.key.as_str()).or_default() += 1;
    }
    let new: Vec<&Message> = other
        .iter()
        .zip(&replay)
        .filter(|&(m, &again)| match have.get_mut(m.key.as_str()) {
            Some(count) if *count > 0 => {
                *count -= 1;
                false
            }
            _ => !again,
        })
        .map(|(m, _)| m)
        .collect();
    if new.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(ours.len() + theirs.len());
    let mut rest = mine.iter().peekable();
    for m in &new {
        while let Some(own) = rest.next_if(|own| own.at <= m.at) {
            put(&mut out, ours, own);
        }
        put(&mut out, theirs, m);
    }
    for own in rest {
        put(&mut out, ours, own);
    }
    Some((out, new.len() as u64))
}

/// Merge a Legacy file into an Aurora one, on the transcript thread; the
/// number of messages added. The target is replaced in one step and only if
/// it did not grow in the meantime, which another viewer on the same account
/// could do. The source is never written.
pub fn merge_file(source: &Path, target: &Path) -> std::io::Result<u64> {
    if same_path(source, target) {
        return Ok(0);
    }
    let theirs = std::fs::read(source)?;
    crate::disk_io::read(crate::disk_io::Origin::Logs, theirs.len());
    let group = target
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| undated(stem).ends_with(names::GROUP_SUFFIX));
    for _ in 0..2 {
        let ours = match std::fs::read(target) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        crate::disk_io::read(crate::disk_io::Origin::Logs, ours.len());
        let Some((text, added)) = merged(&ours, &theirs, group) else {
            return Ok(0);
        };
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let still = || target.metadata().map_or(0, |m| m.len()) == ours.len() as u64;
        if crate::atomic_file::write_if(target, &text, still)? {
            crate::disk_io::written(crate::disk_io::Origin::Logs, text.len());
            return Ok(added);
        }
    }
    Err(std::io::Error::other("the transcript keeps changing"))
}

/// Where to read from.
pub enum Sources {
    /// Folders chosen in the import window.
    Folders(Vec<PathBuf>),
    /// Everything found for the account (the import at login).
    Find { apps: Vec<PathBuf>, account: Account },
}

/// An import to run.
pub struct Request {
    pub sources: Sources,
    /// The folder of the account in Aurora.
    pub dir: PathBuf,
    pub dated: bool,
    /// What was imported, to leave alone at login the files that did not
    /// change: reading them all at every login would be wasted.
    pub state: PathBuf,
    pub skip_unchanged: bool,
}

/// What an import did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    /// Messages added.
    pub added: u64,
    /// The Aurora files that changed, by the stem of their name.
    pub changed: BTreeSet<String>,
    /// Files that could not be merged; they stay as they were.
    pub failed: usize,
    /// Text files without any timestamp, not imported.
    pub left_out: usize,
    /// Started by the login, not by the import window.
    pub at_login: bool,
}

/// An import under way.
pub struct Running {
    pub done: Arc<AtomicUsize>,
    pub total: Arc<AtomicUsize>,
    pub report: Receiver<Report>,
    cancel: Arc<AtomicBool>,
}

impl Running {
    /// Stop after the file being merged: what is merged stays, importing
    /// again completes it.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

type Signature = (u64, u64);

/// Size and modification time.
fn signature(path: &Path) -> Option<Signature> {
    let meta = path.metadata().ok()?;
    Some((meta.len(), meta.modified().ok().map_or(0, |t| unix(t) as u64)))
}

fn load_state(path: &Path) -> HashMap<String, Signature> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_state(path: &Path, state: &HashMap<String, Signature>) {
    let saved = serde_json::to_vec(state).map_err(std::io::Error::other).and_then(|bytes| {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, bytes)
    });
    if let Err(e) = saved {
        log::warn!("conversation transcripts: the import's state is not saved ({e})");
    }
}

/// Run an import on a thread of its own, which reads the folders and hands
/// the transcript thread one file at a time. None when it cannot start.
pub fn start(port: store::MergePort, request: Request) -> Option<Running> {
    let (tx, report) = crossbeam_channel::bounded(1);
    let done = Arc::new(AtomicUsize::new(0));
    let total = Arc::new(AtomicUsize::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    let (d, t, c) = (done.clone(), total.clone(), cancel.clone());
    std::thread::Builder::new()
        .name("aurora-chatlog-import".into())
        .spawn(move || run(&port, request, &d, &t, &c, &tx))
        .map_err(|e| log::warn!("conversation transcripts: no import thread ({e})"))
        .ok()?;
    Some(Running {
        done,
        total,
        report,
        cancel,
    })
}

fn run(port: &store::MergePort, request: Request, done: &AtomicUsize, total: &AtomicUsize, cancel: &AtomicBool, report: &Sender<Report>) {
    let folders: Vec<PathBuf> = match request.sources {
        Sources::Folders(folders) => folders.into_iter().filter(|f| !same_path(f, &request.dir)).collect(),
        Sources::Find { apps, account } => find(&apps, &account, &request.dir)
            .into_iter()
            .filter(|found| !found.same)
            .map(|found| found.dir)
            .collect(),
    };
    let sources: Vec<PathBuf> = folders.iter().flat_map(|folder| transcripts(folder)).collect();
    let left_out = folders.iter().map(|folder| left_out(folder)).sum();
    let spelled = spellings(&request.dir, &sources);
    let plan: Vec<(PathBuf, PathBuf)> = sources
        .into_iter()
        .filter_map(|source| Some((target_of(&request.dir, &source, request.dated, &spelled)?, source)))
        .map(|(target, source)| (source, target))
        .collect();
    total.store(plan.len(), Ordering::Relaxed);
    let mut state = load_state(&request.state);
    // files a source of this run creates: the others meant for them are due
    let missing: HashSet<PathBuf> = plan.iter().map(|(_, target)| target).filter(|t| !t.is_file()).cloned().collect();
    let mut out = Report {
        left_out,
        at_login: request.skip_unchanged,
        ..Report::default()
    };
    let mut first_error = None;
    for (source, target) in plan {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        // with its target: Dated files moves where a source goes
        let key = format!("{}\n{}", source.display(), target.display());
        let seen = signature(&source);
        if request.skip_unchanged && seen.is_some() && state.get(&key) == seen.as_ref() && !missing.contains(&target) {
            done.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let Some(merged) = port.merge(source, target.clone()) else {
            break;
        };
        match merged {
            Ok(added) => {
                if let Some(seen) = seen {
                    state.insert(key, seen);
                }
                if added > 0 {
                    out.added += added;
                    if let Some(stem) = target.file_stem().and_then(|s| s.to_str()) {
                        out.changed.insert(stem.to_owned());
                    }
                }
            }
            Err(e) => {
                out.failed += 1;
                first_error.get_or_insert(e);
            }
        }
        done.fetch_add(1, Ordering::Relaxed);
    }
    save_state(&request.state, &state);
    // no folder or file name: they are names of residents and groups, and
    // the viewer's log may be shared
    log::info!(
        "conversation transcripts: import added {} messages to {} files, {} failed, {} left out{}",
        out.added,
        out.changed.len(),
        out.failed,
        out.left_out,
        first_error.map_or(String::new(), |e| format!(" ({e})"))
    );
    let _ = report.send(out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aurora-chatlog-import-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn account(grid: &str) -> Account {
        Account {
            first: "Jane".into(),
            last: "Doe".into(),
            grid: grid.into(),
        }
    }

    fn text(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec()).unwrap().replace("\r\n", "\n")
    }

    fn merge(ours: &str, theirs: &str) -> Option<(String, u64)> {
        merged(ours.as_bytes(), theirs.as_bytes(), false).map(|(out, added)| (text(&out), added))
    }

    fn merge_group(ours: &str, theirs: &str) -> Option<(String, u64)> {
        merged(ours.as_bytes(), theirs.as_bytes(), true).map(|(out, added)| (text(&out), added))
    }

    #[test]
    fn finds_the_account_in_the_default_and_in_a_chosen_folder() {
        let root = temp();
        let target = root.join("aurora");
        std::fs::create_dir_all(&target).unwrap();
        // one viewer with the default folder
        let app = root.join("one");
        std::fs::create_dir_all(app.join("jane_doe")).unwrap();
        std::fs::write(app.join("jane_doe/Bob Ray.txt"), "[2026/10/12 14:05]  Bob Ray: hi\n").unwrap();
        // another account's
        std::fs::create_dir_all(app.join("other_one")).unwrap();
        std::fs::write(app.join("other_one/Bob Ray.txt"), "[2026/10/12 14:05]  Bob Ray: hi\n").unwrap();
        // a second viewer with InstantMessageLogPath set
        let chosen = root.join("my logs");
        std::fs::create_dir_all(chosen.join("jane_doe")).unwrap();
        std::fs::write(chosen.join("jane_doe/Ann Lee.txt"), "[2026/10/12 14:05]  Ann Lee: hi\n").unwrap();
        let other = root.join("two");
        std::fs::create_dir_all(other.join("jane_doe")).unwrap();
        std::fs::write(
            other.join("jane_doe").join(ACCOUNT_SETTINGS),
            format!(
                "<llsd><map><key>InstantMessageLogPath</key><map><key>Type</key><string>String</string><key>Value</key><string>{}</string></map></map></llsd>",
                chosen.display()
            ),
        )
        .unwrap();
        // a third that shares that folder: found once
        let third = root.join("three");
        std::fs::create_dir_all(third.join("jane_doe")).unwrap();
        std::fs::copy(
            other.join("jane_doe").join(ACCOUNT_SETTINGS),
            third.join("jane_doe").join(ACCOUNT_SETTINGS),
        )
        .unwrap();
        let found = find(&[app.clone(), other, third, root.join("missing")], &account("agni"), &target);
        let dirs: Vec<&Path> = found.iter().map(|f| f.dir.as_path()).collect();
        assert_eq!(dirs, [app.join("jane_doe").as_path(), chosen.join("jane_doe").as_path()]);
        assert!(found.iter().all(|f| f.files == 1 && f.bytes > 0 && !f.same));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_log_folder_that_is_gone_falls_back_like_the_login_does() {
        let root = temp();
        std::fs::write(
            root.join(ACCOUNT_SETTINGS),
            "<llsd><map><key>InstantMessageLogPath</key><map><key>Value</key><string>/no/such/folder</string></map></map></llsd>",
        )
        .unwrap();
        assert_eq!(log_root(Path::new("app"), &root), Path::new("app"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_chosen_folder_may_be_the_viewers_own() {
        let root = temp();
        let inner = root.join("jane_doe");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(inner.join("Bob.txt"), "[2026/10/12 14:05]  Bob: hi\n").unwrap();
        let found = inspect_chosen(&root, &account("agni"), Path::new("elsewhere"));
        assert_eq!((found.dir, found.files), (inner, 1));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_stamp_on_a_later_line_is_a_transcript() {
        let dir = temp();
        std::fs::write(dir.join("Blank.txt"), "\n[2026/10/12 14:05]  Bob: hi\n").unwrap();
        std::fs::write(dir.join("Late.txt"), "no stamp yet\r\n[2026/10/12 14:05]  Bob: hi\r\n").unwrap();
        std::fs::write(dir.join("None.txt"), "Bob: no stamp at all\n").unwrap();
        std::fs::write(dir.join("search_history.txt"), "<llsd><array /></llsd>\n").unwrap();
        std::fs::write(dir.join("teleport_history.txt"), "{\"a\":1}\n").unwrap();
        std::fs::write(dir.join("empty.txt"), "").unwrap();
        let names: Vec<String> = transcripts(&dir)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["Blank.txt", "Late.txt"]);
        // only what may be a conversation is told: not the settings files
        assert_eq!(left_out(&dir), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_transcripts_are_taken() {
        let dir = temp();
        std::fs::write(dir.join("search_history.txt"), "<llsd><array /></llsd>\n").unwrap();
        std::fs::write(dir.join("teleport_history.txt"), "{\"a\":1}\n").unwrap();
        std::fs::write(dir.join("empty.txt"), "").unwrap();
        std::fs::write(dir.join("old.txt.backup"), "[2026/10/12 14:05]  A: b\n").unwrap();
        std::fs::write(dir.join("Bob.txt"), "[2026/10/12 14:05]  Bob: hi\n").unwrap();
        std::fs::write(dir.join("Sec.txt"), "\u{feff}[2026/10/12 14:05:09]  Bob: hi\n").unwrap();
        std::fs::write(dir.join("Pm.txt"), "[3:05 PM]  Bob: hi\n").unwrap();
        // the nearby chat's file counts whatever it starts with
        std::fs::write(dir.join("chat.txt"), "a line without a stamp\n").unwrap();
        std::fs::write(dir.join("chat-2026-10-12.txt"), "\n").unwrap();
        let names: Vec<String> = transcripts(&dir)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["Bob.txt", "Pm.txt", "Sec.txt", "chat-2026-10-12.txt", "chat.txt"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dates_in_file_names() {
        assert_eq!(undated("chat-2026-10-12"), "chat");
        assert_eq!(undated("chat"), "chat");
        assert_eq!(undated("Jane Doe-2026-10"), "Jane Doe");
        assert_eq!(undated("Club (group)-2026-10"), "Club (group)");
        assert_eq!(undated("Jane Doe"), "Jane Doe");
        assert_eq!(undated("-2026-10"), "-2026-10");
        // only the nearby chat's file has a day; a conference has no date
        assert_eq!(undated("Jane-2026-10-12"), "Jane-2026-10-12");
        assert_eq!(
            undated("Jane Doe Conference 2010_11_19 03_43 f0f4"),
            "Jane Doe Conference 2010_11_19 03_43 f0f4"
        );
        assert_eq!(undated("ジェーン-2026-10"), "ジェーン");
        let target = |name: &str, dated| target_of(Path::new("d"), Path::new(name), dated, &Spellings::new()).unwrap();
        assert_eq!(target("Jane-2026-10.txt", false), Path::new("d/Jane.txt"));
        assert_eq!(target("Jane-2026-10.txt", true), Path::new("d/Jane-2026-10.txt"));
        assert_eq!(target("Jane.txt", true), Path::new("d/Jane.txt"));
    }

    /// Runs an import of `from` at login to its end.
    fn import_at_login(from: &Path, to: &Path, dated: bool) -> Report {
        let worker = store::Worker::start().unwrap();
        let request = Request {
            sources: Sources::Folders(vec![from.to_path_buf()]),
            dir: to.to_path_buf(),
            dated,
            state: to.join("state.json"),
            skip_unchanged: true,
        };
        let running = start(worker.merge_port(), request).unwrap();
        running.report.recv().unwrap()
    }

    #[test]
    fn at_login_a_source_is_read_again_when_its_target_is_not_the_same() {
        let (from, to) = (temp(), temp());
        let line = |day| format!("[2026/10/{day:02} 12:00]  Jane Doe: hello {day}\n");
        std::fs::write(from.join("Bob-2026-09.txt"), line(1)).unwrap();
        std::fs::write(from.join("Bob-2026-10.txt"), line(2)).unwrap();

        assert_eq!(import_at_login(&from, &to, true).added, 2);
        assert_eq!(import_at_login(&from, &to, true).added, 0);
        // Dated files turned off: both join Bob.txt
        assert_eq!(import_at_login(&from, &to, false).added, 2);
        assert_eq!(import_at_login(&from, &to, false).added, 0);
        // the file deleted: the first source to create it does not end the run
        std::fs::remove_file(to.join("Bob.txt")).unwrap();
        assert_eq!(import_at_login(&from, &to, false).added, 2);
        std::fs::remove_dir_all(from).unwrap();
        std::fs::remove_dir_all(to).unwrap();
    }

    #[test]
    fn a_username_file_is_the_residents_file() {
        let none = Spellings::new();
        assert_eq!(resident_name("jane_doe", &none), "Jane Doe");
        assert_eq!(resident_name("jane_doe-2026-10", &none), "Jane Doe-2026-10");
        assert_eq!(resident_name("janedoe", &none), "janedoe");
        assert_eq!(resident_name("Jane Doe", &none), "Jane Doe");
        assert_eq!(resident_name("chat", &none), "chat");
        assert_eq!(resident_name("a_b_c", &none), "a_b_c");
        assert_eq!(resident_name("foo_bar (group)", &none), "foo_bar (group)");
        let target = target_of(Path::new("d"), Path::new("jane_doe.txt"), false, &none);
        assert_eq!(target.unwrap(), Path::new("d/Jane Doe.txt"));
    }

    #[test]
    fn a_username_takes_the_capitals_known_for_it() {
        let dir = temp();
        std::fs::write(dir.join("JaneDoe.txt"), "").unwrap();
        std::fs::write(dir.join("Club (group).txt"), "").unwrap();
        let sources = [
            PathBuf::from("legacy/Jane McDoe-2026-10.txt"),
            PathBuf::from("legacy/janedoe.txt"),
            PathBuf::from("legacy/JANEDOE.txt"),
        ];
        let spelled = spellings(&dir, &sources);
        assert_eq!(resident_name("janedoe", &spelled), "JaneDoe");
        assert_eq!(resident_name("janedoe-2026-10", &spelled), "JaneDoe-2026-10");
        assert_eq!(resident_name("jane_mcdoe", &spelled), "Jane McDoe");
        // written with capitals, or not a resident's: as it is
        assert_eq!(resident_name("JANEDOE", &spelled), "JANEDOE");
        assert_eq!(resident_name("club (group)", &spelled), "club (group)");
        assert_eq!(resident_name("chat", &[("chat".to_owned(), "Chat".to_owned())].into()), "chat");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_groups_doubled_name_is_the_groups_file() {
        assert_eq!(once_group("Club (group) (group)"), "Club (group)");
        assert_eq!(once_group("Club (group) (group) (group)"), "Club (group)");
        assert_eq!(once_group("Club (group)"), "Club (group)");
        assert_eq!(once_group("Club (group) (group)-2026-10"), "Club (group)-2026-10");
        assert_eq!(once_group("Jane Doe"), "Jane Doe");
        let target = target_of(
            Path::new("d"),
            Path::new("Club (group) (group)-2026-10.txt"),
            false,
            &Spellings::new(),
        );
        assert_eq!(target.unwrap(), Path::new("d/Club (group).txt"));
    }

    #[test]
    fn a_new_conversation_is_kept_as_written() {
        let theirs = "[2026/10/12 14:05]  Bob: one\n second line\n\n after a gap\n[3:06 PM]  Bob: two\nno stamp at all\n";
        let (out, added) = merge("", theirs).unwrap();
        assert_eq!(out, theirs);
        assert_eq!(added, 3);
    }

    #[test]
    fn merging_goes_by_time_and_a_second_run_adds_nothing() {
        let ours = "[2026/10/12 14:05]  Bob: a\n[2026/10/14 09:00]  Bob: d\n";
        let theirs =
            "[2026/10/11 10:00]  Bob: 0\n[2026/10/12 14:05]  Bob: a\n[2026/10/13 08:00]  Bob: c\n more of c\n[2026/10/15 08:00]  Bob: e\n";
        let (out, added) = merge(ours, theirs).unwrap();
        assert_eq!(
            out,
            "[2026/10/11 10:00]  Bob: 0\n[2026/10/12 14:05]  Bob: a\n[2026/10/13 08:00]  Bob: c\n more of c\n[2026/10/14 09:00]  Bob: d\n[2026/10/15 08:00]  Bob: e\n"
        );
        assert_eq!(added, 3);
        assert_eq!(merge(&out, theirs), None);
    }

    #[test]
    fn a_message_said_twice_counts_twice() {
        let theirs = "[2026/10/12 14:05]  Bob: ok\n[2026/10/12 14:05]  Bob: ok\n";
        // once here: the second one is new
        let (out, added) = merge("[2026/10/12 14:05]  Bob: ok\n", theirs).unwrap();
        assert_eq!(added, 1);
        assert_eq!(out, theirs);
        assert_eq!(merge(&out, theirs), None);
        // the same text from someone else or at another minute is another
        // message
        assert_eq!(merge(theirs, "[2026/10/12 14:05]  Ann: ok\n").unwrap().1, 1);
        assert_eq!(merge(theirs, "[2026/10/12 14:06]  Bob: ok\n").unwrap().1, 1);
    }

    const SAID: &str = "[2026/10/12 14:05]  Bob: hi\n[2026/10/12 14:06]  Ann: hello\n";

    #[test]
    fn group_history_written_again_is_left_out() {
        let again = "[2026/11/05 07:45]  Bob: hi\n[2026/11/05 07:45]  Ann: hello\n";
        // what the Aurora file already has
        assert_eq!(merge_group(SAID, again), None);
        // a first import: the source says it twice itself
        let source = format!("{SAID}{again}");
        let (out, added) = merge_group("", &source).unwrap();
        assert_eq!(added, 2);
        assert_eq!(out, SAID);
        // and a second run changes nothing
        assert_eq!(merge_group(&out, &source), None);
        // a resident's file is never touched: they said it twice
        assert_eq!(merge(SAID, again).unwrap().1, 2);
    }

    #[test]
    fn a_group_line_said_again_is_kept_unless_the_whole_minute_is_old() {
        // alone in its minute
        assert_eq!(merge_group(SAID, "[2026/11/05 07:45]  Bob: hi\n").unwrap().1, 1);
        // beside something new, the minute is real chat
        let mixed = "[2026/11/05 07:45]  Bob: hi\n[2026/11/05 07:45]  Ann: hello\n[2026/11/05 07:45]  Ann: new\n";
        assert_eq!(merge_group(SAID, mixed).unwrap().1, 3);
        // the same words from someone else are new
        let other = "[2026/11/05 07:45]  Cid: hi\n[2026/11/05 07:45]  Ann: hello\n";
        assert_eq!(merge_group(SAID, other).unwrap().1, 2);
        // said twice in one minute and never before: not a replay
        let twice = "[2026/10/12 14:05]  Bob: ok\n[2026/10/12 14:05]  Bob: ok\n";
        assert_eq!(merge_group("", twice).unwrap().1, 2);
    }

    #[test]
    fn only_a_groups_file_leaves_history_out() {
        let dir = temp();
        let again = "[2026/11/05 07:45]  Bob: hi\n[2026/11/05 07:45]  Ann: hello\n";
        let source = dir.join("source.txt");
        std::fs::write(&source, format!("{SAID}{again}")).unwrap();
        for (name, added) in [("Club (group).txt", 2), ("Club (group)-2026-11.txt", 2), ("Jane Doe.txt", 4)] {
            assert_eq!(merge_file(&source, &dir.join(name)).unwrap(), added, "{name}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_same_minute_in_another_form_is_the_same_message() {
        let ours = "[2026/10/12 14:05]  Bob: hi\n";
        // seconds, FSSecondsinChatTimestamps
        assert_eq!(merge(ours, "[2026/10/12 14:05:33]  Bob: hi\n"), None);
        // a 12 hour clock, Use24HourClock off
        assert_eq!(merge(ours, "[2026/10/12 2:05 PM]  Bob: hi\n"), None);
        // no zero in front
        assert_eq!(merge("[2026/10/02 04:05]  Bob: hi\n", "[2026/10/2 4:05]  Bob: hi\n"), None);
    }

    #[test]
    fn lines_without_a_date_stay_where_they_are_in_their_file() {
        // without LogTimestampDate the date is unknown: they go first
        let (out, added) = merge("[2026/10/12 14:05]  Bob: new\n", "[14:05]  Bob: old\n").unwrap();
        assert_eq!(out, "[14:05]  Bob: old\n[2026/10/12 14:05]  Bob: new\n");
        assert_eq!(added, 1);
        assert_eq!(merge(&out, "[14:05]  Bob: old\n"), None);
        // after a dated line, they follow it
        let theirs = "[2026/10/13 08:00]  Bob: b\na system line\n";
        let (out, _) = merge("[2026/10/14 09:00]  Bob: c\n", theirs).unwrap();
        assert_eq!(out, "[2026/10/13 08:00]  Bob: b\na system line\n[2026/10/14 09:00]  Bob: c\n");
    }

    #[test]
    fn our_order_is_never_changed() {
        // a clock that went back: both of ours stay, in their order
        let ours = "[2026/10/14 09:00]  Bob: later\n[2026/10/12 09:00]  Bob: earlier\n";
        let (out, _) = merge(ours, "[2026/10/13 09:00]  Bob: between\n").unwrap();
        let (l, e) = (out.find("later").unwrap(), out.find("earlier").unwrap());
        assert!(l < e, "{out}");
    }

    #[test]
    fn line_ends_and_odd_bytes_survive() {
        let theirs = b"[2026/10/12 14:05]  Bob: caf\xE9\r\n more\r\n";
        let (out, added) = merged(b"", theirs, false).unwrap();
        assert_eq!(added, 1);
        // the invalid byte stays; the ends are ours
        assert_eq!(
            out,
            [
                b"[2026/10/12 14:05]  Bob: caf\xE9".as_slice(),
                store::EOL.as_bytes(),
                b" more",
                store::EOL.as_bytes()
            ]
            .concat()
        );
        assert!(merged(&out, theirs, false).is_none());
        // a byte order mark is not part of the first message
        assert!(merged(b"[2026/10/12 14:05]  Bob: a\n", b"\xEF\xBB\xBF[2026/10/12 14:05]  Bob: a\n", false).is_none());
    }

    #[test]
    fn a_file_is_merged_in_place_and_the_source_is_not_touched() {
        let dir = temp();
        let (source, target) = (dir.join("legacy/Bob.txt"), dir.join("aurora/x/Bob.txt"));
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "[2026/10/12 14:05]  Bob: a\n").unwrap();
        assert_eq!(merge_file(&source, &target).unwrap(), 1);
        assert_eq!(merge_file(&source, &target).unwrap(), 0);
        assert_eq!(text(&std::fs::read(&target).unwrap()), "[2026/10/12 14:05]  Bob: a\n");
        assert_eq!(std::fs::read(&source).unwrap(), b"[2026/10/12 14:05]  Bob: a\n");
        assert_eq!(std::fs::read_dir(target.parent().unwrap()).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_linked_to_the_legacy_one_is_left_alone() {
        let dir = temp();
        let legacy = dir.join("legacy");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("Bob.txt"), "[2026/10/12 14:05]  Bob: a\n").unwrap();
        let link = dir.join("agni");
        std::os::unix::fs::symlink(&legacy, &link).unwrap();
        assert!(inspect(&legacy, &link).same);
        assert_eq!(merge_file(&legacy.join("Bob.txt"), &link.join("Bob.txt")).unwrap(), 0);
        assert_eq!(std::fs::read(legacy.join("Bob.txt")).unwrap(), b"[2026/10/12 14:05]  Bob: a\n");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_failure_leaves_the_file_as_it_was() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp();
        let (source, aurora) = (dir.join("legacy.txt"), dir.join("aurora"));
        std::fs::create_dir_all(&aurora).unwrap();
        let target = aurora.join("Bob.txt");
        std::fs::write(&target, "[2026/10/14 09:00]  Bob: mine\n").unwrap();
        std::fs::write(&source, "[2026/10/12 14:05]  Bob: a\n").unwrap();
        // no temporary file can be created in a read-only folder
        std::fs::set_permissions(&aurora, std::fs::Permissions::from_mode(0o555)).unwrap();
        let merged = merge_file(&source, &target);
        std::fs::set_permissions(&aurora, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(merged.is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"[2026/10/14 09:00]  Bob: mine\n");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
