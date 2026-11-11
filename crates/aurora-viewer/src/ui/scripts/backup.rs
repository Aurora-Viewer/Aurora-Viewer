//! A copy of each script being edited with unsaved changes, so that a crash
//! or a lost connection does not lose them: the next login offers them back
//! (`recovery.rs`). Port of the backups of LLScriptEdContainer
//! (`onBackupTimer`, `getBackupFileName`, the removals in the destructor and
//! in `callbackLSLCompileSucceeded` / `callbackLSLCompileFailed`,
//! indra/newview/llpreviewscript.cpp, originally LGPL 2.1).
//!
//! The Legacy Viewer writes a plain `.lslbackup` file in the system's
//! temporary folder, named after the agent, the item and a CRC of its id,
//! every 60 seconds once the text changed. Here the copies live in the
//! account's own folder (`settings::account_dir`, like its other files),
//! each one sealed with the key the disk cache uses (`crate::cache::key`,
//! kept in the system's credential store): the name and the text of the
//! script are inside, the file name is derived from that key and says
//! nothing. They are not in the cache folder, whose size limit and Clear
//! Cache would remove them. A copy is written at most every
//! [`INTERVAL`], on a thread of its own: the frame only compares a hash.
//!
//! Which scripts: the ones whose editor may edit them (`Editor::can_edit`,
//! the copy and modify rights of `app/scripts.rs::permissions`), as the
//! Legacy Viewer only backs up an editor that has changed, which needs
//! editing enabled. A copy goes when its editor closes or saves (the text
//! the server holds is the one shown), and stays when the connection is
//! lost, like the Legacy Viewer's (`gAgent.getRegion()` null).

use aurora_net::scripts::ScriptTarget;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The folder of the copies in the account's folder.
pub const FOLDER: &str = "script-backups";
const EXTENSION: &str = "bak";
/// How often a changed text is copied again. The Legacy Viewer's timer runs
/// every 60 seconds; the copy costs nothing to the frame here, so less of
/// the work is lost.
pub const INTERVAL: Duration = Duration::from_secs(10);

/// The folder of `agent`'s copies.
pub fn folder(agent: &Uuid) -> PathBuf {
    crate::settings::account_dir(agent).join(FOLDER)
}

/// What a copy holds.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Copy {
    /// The script's name when it was copied.
    pub name: String,
    pub source: String,
    /// When it was copied (seconds since 1970).
    pub saved: i64,
}

/// One id per script: an inventory script's item, or a script in an
/// object (its item id is only unique within that object).
fn key(target: ScriptTarget) -> Uuid {
    Uuid::from_u128(target.object.as_u128() ^ target.item.as_u128())
}

/// The file of `target`'s copy in `dir`: the same one for every copy of
/// that script (the Legacy Viewer's CRC of the item id), a name the
/// installation's key derives from it.
pub fn file(dir: &Path, target: ScriptTarget) -> PathBuf {
    dir.join(format!("{}.{EXTENSION}", crate::cache::keyed_name(&key(target))))
}

/// Whether `path` is the file of a copy.
pub fn is_copy(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == EXTENSION)
}

pub fn encode(copy: &Copy) -> std::io::Result<Vec<u8>> {
    crate::cache::key().seal(&serde_json::to_vec(copy)?)
}

/// A copy read back; `None` for a damaged file, one of an older key (the
/// credential store reset), or anything else.
pub fn decode(file: Vec<u8>) -> Option<Copy> {
    let plain = crate::cache::key().open(file).ok()?;
    serde_json::from_slice(&plain).ok()
}

#[derive(Debug, PartialEq, Eq)]
pub enum Job {
    Write(PathBuf, Copy),
    Remove(PathBuf),
}

enum Message {
    Job(Job),
    /// Answered once every job before it is done.
    Flush(crossbeam_channel::Sender<()>),
}

static WORKER: std::sync::OnceLock<Option<crossbeam_channel::Sender<Message>>> = std::sync::OnceLock::new();

fn worker() -> Option<&'static crossbeam_channel::Sender<Message>> {
    WORKER
        .get_or_init(|| {
            let (tx, rx) = crossbeam_channel::unbounded::<Message>();
            let spawned = std::thread::Builder::new()
                .name("aurora-script-backups".into())
                .spawn(move || {
                    for message in rx {
                        match message {
                            Message::Job(job) => apply(job),
                            Message::Flush(done) => {
                                let _ = done.send(());
                            }
                        }
                    }
                });
            match spawned {
                Ok(_) => Some(tx),
                Err(e) => {
                    log::warn!("script backups: no thread ({e}); copies written in place");
                    None
                }
            }
        })
        .as_ref()
}

/// Hand the jobs to the copies' thread, in order (a write, then the removal
/// of the same file, must stay in that order).
pub fn run(jobs: Vec<Job>) {
    if jobs.is_empty() {
        return;
    }
    let worker = worker();
    for job in jobs {
        match worker {
            Some(tx) => {
                if let Err(e) = tx.send(Message::Job(job)) {
                    if let Message::Job(job) = e.into_inner() {
                        apply(job);
                    }
                }
            }
            None => apply(job),
        }
    }
}

/// Wait, at most `limit`, for the jobs handed over so far: at exit, so that
/// the copy of an editor just closed does not outlive the process.
pub fn flush(limit: Duration) {
    // no thread yet: nothing was handed over
    let Some(Some(tx)) = WORKER.get() else { return };
    let (done, wait) = crossbeam_channel::bounded(1);
    if tx.send(Message::Flush(done)).is_ok() && wait.recv_timeout(limit).is_err() {
        log::warn!("script backups: still being written at exit");
    }
}

/// One job, on the copies' thread. The copy is flushed to the disk before
/// it replaces the older one (`atomic_file::write`): it must survive the
/// crash it is for.
pub fn apply(job: Job) {
    match job {
        Job::Write(path, copy) => {
            let written = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| encode(&copy))
                .and_then(|bytes| crate::atomic_file::write(&path, &bytes));
            match written {
                Ok(()) => log::info!("script backup: unsaved changes copied ({} bytes)", copy.source.len()),
                Err(e) => log::warn!("script backup: not written: {e}"),
            }
        }
        Job::Remove(path) => match std::fs::remove_file(&path) {
            Ok(()) => log::info!("script backup: removed"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("script backup: not removed: {e}"),
        },
    }
}

/// What the book needs of an open editor.
pub struct Doc<'a> {
    pub target: ScriptTarget,
    pub name: &'a str,
    pub source: &'a str,
    /// The text differs from the one the server holds.
    pub unsaved: bool,
    /// Loaded, and the agent may edit it.
    pub editable: bool,
}

/// The copies this session keeps for its open editors.
#[derive(Default)]
pub struct Book {
    owned: HashMap<ScriptTarget, Entry>,
    next: Option<Instant>,
}

struct Entry {
    path: PathBuf,
    /// The hash of the name and the text last copied.
    written: u64,
}

fn digest(name: &str, source: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut h);
    source.hash(&mut h);
    h.finish()
}

impl Book {
    /// The jobs for the editors open now. The copy of an editor closed
    /// since, or whose text is the saved one again, goes at once; a
    /// changed text is copied every [`INTERVAL`], or now with `force`.
    /// `dir` is the account's folder of copies; `None` writes nothing (no
    /// agent yet, the demo).
    pub fn plan<'a>(&mut self, dir: Option<&Path>, docs: impl IntoIterator<Item = Doc<'a>>, now: Instant, force: bool) -> Vec<Job> {
        let docs: Vec<Doc> = docs.into_iter().collect();
        let mut jobs = Vec::new();
        self.owned.retain(|target, entry| {
            let keep = docs.iter().any(|d| d.target == *target && d.unsaved);
            if !keep {
                jobs.push(Job::Remove(entry.path.clone()));
            }
            keep
        });
        let due = force || self.next.is_none_or(|next| now >= next);
        if !due {
            return jobs;
        }
        self.next = Some(now + INTERVAL);
        let Some(dir) = dir else { return jobs };
        let saved = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        for d in docs.iter().filter(|d| d.unsaved && d.editable) {
            let hash = digest(d.name, d.source);
            if self.owned.get(&d.target).is_some_and(|e| e.written == hash) {
                continue;
            }
            let path = file(dir, d.target);
            jobs.push(Job::Write(
                path.clone(),
                Copy {
                    name: d.name.to_owned(),
                    source: d.source.to_owned(),
                    saved,
                },
            ));
            self.owned.insert(d.target, Entry { path, written: hash });
        }
        jobs
    }

    /// The connection is lost: the copies stay on the disk for the next
    /// login, whatever their editors do now.
    pub fn release(&mut self) {
        self.owned.clear();
    }

    /// The file is the copy of an editor open now.
    pub fn owns(&self, path: &Path) -> bool {
        self.owned.values().any(|e| e.path == path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(object: u128, item: u128) -> ScriptTarget {
        ScriptTarget {
            object: Uuid::from_u128(object),
            item: Uuid::from_u128(item),
        }
    }

    fn doc(target: ScriptTarget, source: &str, unsaved: bool, editable: bool) -> Doc<'_> {
        Doc {
            target,
            name: "Door",
            source,
            unsaved,
            editable,
        }
    }

    fn writes(jobs: &[Job]) -> usize {
        jobs.iter().filter(|j| matches!(j, Job::Write(..))).count()
    }

    fn removes(jobs: &[Job]) -> usize {
        jobs.iter().filter(|j| matches!(j, Job::Remove(..))).count()
    }

    #[test]
    fn each_script_has_one_file_that_names_neither_its_id_nor_its_name() {
        let dir = Path::new("backups");
        let inventory = target(0, 7);
        let in_object = target(9, 7);
        assert_eq!(file(dir, inventory), file(dir, inventory), "the same file for every copy");
        assert_ne!(file(dir, inventory), file(dir, in_object), "the same item id in an object is another script");
        let name = file(dir, inventory).file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
        assert!(!name.contains(&Uuid::from_u128(7).to_string()) && !name.contains("Door"));
        assert!(is_copy(&file(dir, inventory)));
        assert!(!is_copy(Path::new("backups/notes.txt")));
    }

    #[test]
    fn a_copy_is_sealed_and_reads_back() {
        let copy = Copy {
            name: "Door".into(),
            source: "default { state_entry() { llSay(0, \"unsaved\"); } }".into(),
            saved: 1_700_000_000,
        };
        let sealed = encode(&copy).expect("sealed");
        let text = String::from_utf8_lossy(&sealed);
        assert!(!text.contains("unsaved") && !text.contains("Door"), "neither the text nor the name in clear");
        assert_eq!(decode(sealed.clone()), Some(copy));
        let mut damaged = sealed;
        let last = damaged.len() - 1;
        damaged[last] ^= 1;
        assert_eq!(decode(damaged), None);
        assert_eq!(decode(b"{\"name\":\"x\",\"source\":\"y\",\"saved\":0}".to_vec()), None, "a plain file is refused");
    }

    #[test]
    fn a_changed_text_is_copied_once_per_interval_and_only_when_it_changed() {
        let dir = Path::new("backups");
        let t = target(0, 1);
        let mut book = Book::default();
        let start = Instant::now();
        assert!(book.plan(Some(dir), [doc(t, "a", false, true)], start, false).is_empty(), "nothing unsaved");
        let jobs = book.plan(Some(dir), [doc(t, "ab", true, true)], start + INTERVAL, false);
        assert_eq!(writes(&jobs), 1);
        assert!(book.owns(&file(dir, t)));
        // typing goes on: no copy before the interval
        assert!(book.plan(Some(dir), [doc(t, "abc", true, true)], start + INTERVAL + Duration::from_secs(1), false).is_empty());
        let jobs = book.plan(Some(dir), [doc(t, "abc", true, true)], start + INTERVAL * 2, false);
        assert!(matches!(&jobs[..], [Job::Write(path, copy)] if *path == file(dir, t) && copy.source == "abc" && copy.name == "Door"));
        // the same text: not written again
        assert!(book.plan(Some(dir), [doc(t, "abc", true, true)], start + INTERVAL * 3, false).is_empty());
        // forced (the connection is lost): at once
        assert_eq!(writes(&book.plan(Some(dir), [doc(t, "abcd", true, true)], start + INTERVAL * 3, true)), 1);
    }

    #[test]
    fn only_an_editable_script_is_copied() {
        let dir = Path::new("backups");
        let mut book = Book::default();
        let jobs = book.plan(Some(dir), [doc(target(0, 1), "changed", true, false)], Instant::now(), true);
        assert!(jobs.is_empty(), "no modify, read only, not loaded: no copy");
        assert!(book.plan(None, [doc(target(0, 1), "changed", true, true)], Instant::now(), true).is_empty(), "no folder: nothing");
    }

    #[test]
    fn the_copy_goes_on_save_and_close_and_stays_after_a_disconnection() {
        let dir = Path::new("backups");
        let (a, b) = (target(0, 1), target(5, 2));
        let mut book = Book::default();
        let now = Instant::now();
        assert_eq!(writes(&book.plan(Some(dir), [doc(a, "x", true, true), doc(b, "y", true, true)], now, true)), 2);
        // a saved: its text is the server's again
        let jobs = book.plan(Some(dir), [doc(a, "x", false, true), doc(b, "y", true, true)], now, false);
        assert_eq!(jobs, vec![Job::Remove(file(dir, a))]);
        // b closed (saved, discarded): its editor is gone
        assert_eq!(book.plan(Some(dir), [], now, false), vec![Job::Remove(file(dir, b))]);
        // the connection lost: the copy stays when the editor closes later
        assert_eq!(writes(&book.plan(Some(dir), [doc(a, "z", true, true)], now, true)), 1);
        book.release();
        assert!(!book.owns(&file(dir, a)));
        assert_eq!(removes(&book.plan(Some(dir), [], now, false)), 0);
    }
}
