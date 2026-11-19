//! What the build script declares to Cargo, on a made-up layout and against
//! real repositories: the Git commands of another worktree must change none
//! of the declared files, a commit or a tag of this checkout must change one.

#[path = "../git_watch.rs"]
mod git_watch;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use git_watch::watched_paths;

fn layout(present: &[&str]) -> impl Fn(&Path) -> bool {
    let present: HashSet<PathBuf> = present.iter().map(PathBuf::from).collect();
    move |path| present.contains(path)
}

fn watched(branch: Option<&str>, reflog: bool, present: &[&str]) -> Vec<PathBuf> {
    watched_paths(Path::new("private"), Path::new("common"), branch, reflog, layout(present))
}

fn paths(names: &[&str]) -> Vec<PathBuf> {
    names.iter().map(PathBuf::from).collect()
}

const EVERYTHING: &[&str] = &[
    "private/HEAD",
    "private/logs/HEAD",
    "common/refs/heads/perf/task",
    "common/packed-refs",
    "common/refs",
    "common/refs/tags",
];

#[test]
fn a_checkout_with_a_reflog_declares_nothing_other_worktrees_write() {
    assert_eq!(
        watched(Some("refs/heads/perf/task"), true, EVERYTHING),
        paths(&["private/HEAD", "private/logs/HEAD", "common/refs/tags"])
    );
    // a detached HEAD has no branch to name
    assert_eq!(
        watched(None, true, EVERYTHING),
        paths(&["private/HEAD", "private/logs/HEAD", "common/refs/tags"])
    );
}

#[test]
fn without_a_reflog_the_branch_and_the_packed_references_are_declared() {
    let expected = paths(&[
        "private/HEAD",
        "common/refs/heads/perf/task",
        "common/packed-refs",
        "common/refs/tags",
    ]);
    // reflogs turned off: the file of an earlier time tells nothing
    assert_eq!(watched(Some("refs/heads/perf/task"), false, EVERYTHING), expected);
    // no reflog file
    let no_log: Vec<&str> = EVERYTHING.iter().copied().filter(|path| *path != "private/logs/HEAD").collect();
    assert_eq!(watched(Some("refs/heads/perf/task"), true, &no_log), expected);
    assert_eq!(
        watched(None, true, &no_log),
        paths(&["private/HEAD", "common/packed-refs", "common/refs/tags"])
    );
}

#[test]
fn a_missing_path_is_never_declared() {
    // Cargo would run the script at every build for it.
    assert_eq!(
        watched(
            Some("refs/heads/perf/task"),
            false,
            &["private/HEAD", "common/refs", "common/refs/tags"]
        ),
        paths(&["private/HEAD", "common/refs/tags"])
    );
    // without refs/tags, its parent shows the folder being created
    assert_eq!(
        watched(None, true, &["private/HEAD", "private/logs/HEAD", "common/refs"]),
        paths(&["private/HEAD", "private/logs/HEAD", "common/refs"])
    );
    assert!(watched(Some("refs/heads/perf/task"), true, &[]).is_empty());
}

#[test]
fn this_checkout_declares_only_paths_that_exist() {
    // Empty in a source archive without Git.
    for path in git_watch::watch_list(Path::new(env!("CARGO_MANIFEST_DIR"))) {
        assert!(path.exists(), "{}", path.display());
    }
}

/// Throwaway repositories: `repo` (the main checkout, on `main`, one release
/// tag) and two linked worktrees, `task` (the one observed) and `other`.
/// Every reference starts in `packed-refs` alone.
struct Lab {
    root: PathBuf,
}

impl Lab {
    /// `None` where Git is not installed.
    fn new() -> Option<Self> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        Command::new("git")
            .arg("--version")
            .output()
            .ok()
            .filter(|output| output.status.success())?;
        let root = std::env::temp_dir().join(format!(
            "aurora-version-watch-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("repo")).unwrap();
        fs::write(root.join("gitconfig"), "").unwrap();
        let lab = Self { root };
        lab.git("repo", &["init", "--quiet", "--initial-branch=main"]);
        lab.commit("repo", "one");
        lab.git("repo", &["tag", "-a", "v0.1.0", "-m", "v0.1.0"]);
        lab.commit("repo", "two");
        lab.git("repo", &["worktree", "add", "--quiet", "-b", "perf/task", "../task"]);
        lab.git("repo", &["worktree", "add", "--quiet", "-b", "fix/other", "../other"]);
        // as after a clone or Git's maintenance: every reference in packed-refs
        lab.git("repo", &["pack-refs", "--all"]);
        Some(lab)
    }

    /// Git with the lab's own settings: not the user's identity, signing,
    /// hooks or reference format.
    fn git(&self, dir: &str, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(self.root.join(dir))
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false", "-c", "gc.auto=0"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_DEFAULT_REF_FORMAT", "files")
            .env("GIT_AUTHOR_NAME", "fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} in {dir}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    fn commit(&self, dir: &str, text: &str) {
        fs::write(self.root.join(dir).join(format!("{dir}.txt")), text).unwrap();
        self.git(dir, &["add", "-A"]);
        self.git(dir, &["commit", "--quiet", "-m", text]);
    }

    /// What the build script of `task` declares, with reflogs on (Git's default).
    fn watched(&self) -> Vec<PathBuf> {
        let git_dir = self.git("task", &["rev-parse", "--path-format=absolute", "--git-dir"]);
        let common = self.git("task", &["rev-parse", "--path-format=absolute", "--git-common-dir"]);
        // "HEAD" when it is detached
        let branch = self.git("task", &["rev-parse", "--symbolic-full-name", "HEAD"]);
        let branch = (branch != "HEAD").then_some(branch.as_str());
        let watched = watched_paths(Path::new(&git_dir), Path::new(&common), branch, true, |path| path.exists());
        assert!(watched.iter().any(|path| path.ends_with("logs/HEAD")), "{watched:?}");
        watched
    }

    fn version(&self) -> String {
        self.git(
            "task",
            &["describe", "--tags", "--match", "v[0-9]*.[0-9]*.[0-9]*", "--long", "--abbrev=9"],
        )
    }

    fn packed_refs(&self) -> String {
        fs::read_to_string(self.root.join("repo/.git/packed-refs")).unwrap()
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        // Git's object files are read-only, which stops Windows deleting them.
        fn unlock(dir: &Path) {
            for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                let Ok(metadata) = entry.metadata() else { continue };
                if metadata.is_dir() {
                    unlock(&path);
                } else if metadata.permissions().readonly() {
                    let mut permissions = metadata.permissions();
                    #[allow(clippy::permissions_set_readonly_false)]
                    permissions.set_readonly(false);
                    let _ = fs::set_permissions(&path, permissions);
                }
            }
        }
        if fs::remove_dir_all(&self.root).is_err() {
            unlock(&self.root);
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

/// What Cargo compares: the date of each declared path and, for a folder, of
/// all it holds. The sizes too, for a file system with coarse dates.
fn state(watched: &[PathBuf]) -> Vec<(PathBuf, Option<SystemTime>, u64)> {
    fn visit(path: &Path, out: &mut Vec<(PathBuf, Option<SystemTime>, u64)>) {
        let Ok(metadata) = fs::metadata(path) else {
            out.push((path.to_owned(), None, 0));
            return;
        };
        out.push((
            path.to_owned(),
            metadata.modified().ok(),
            if metadata.is_dir() { 0 } else { metadata.len() },
        ));
        if metadata.is_dir() {
            let mut children: Vec<PathBuf> = fs::read_dir(path)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path())
                .collect();
            children.sort();
            for child in children {
                visit(&child, out);
            }
        }
    }
    let mut out = Vec::new();
    for path in watched {
        visit(path, &mut out);
    }
    out
}

/// Lets a folder's date move on a file system whose clock ticks coarsely.
fn tick() {
    std::thread::sleep(Duration::from_millis(50));
}

#[test]
fn the_git_commands_of_another_worktree_change_no_declared_file() {
    let Some(lab) = Lab::new() else { return };
    // the observed branch gets its own file under refs/heads
    lab.commit("task", "task one");
    let watched = lab.watched();
    let (before, version) = (state(&watched), lab.version());

    // another task commits, on a packed branch then on a loose one
    lab.commit("other", "other one");
    lab.commit("other", "other two");
    // Git's maintenance packs the references: the observed branch loses its file
    lab.git("repo", &["pack-refs", "--all"]);
    assert!(!lab.root.join("repo/.git/refs/heads/perf/task").exists());
    // new-task.ps1 and end-task.ps1 of a third task, whose branch got packed
    lab.git("repo", &["worktree", "add", "--quiet", "-b", "feat/third", "../third"]);
    lab.git("repo", &["pack-refs", "--all"]);
    let packed = lab.packed_refs();
    lab.git("repo", &["worktree", "remove", "../third"]);
    lab.git("repo", &["branch", "-D", "feat/third"]);
    assert_ne!(lab.packed_refs(), packed, "deleting a packed branch rewrites packed-refs");

    assert_eq!(lab.version(), version);
    assert_eq!(state(&watched), before);
}

#[test]
fn a_move_of_this_checkout_changes_a_declared_file() {
    let Some(lab) = Lab::new() else { return };
    let step = |what: &str, act: &dyn Fn()| {
        // as the build script declared them at its last run
        let watched = lab.watched();
        let (before, version) = (state(&watched), lab.version());
        act();
        assert_ne!(lab.version(), version, "{what}");
        assert_ne!(state(&watched), before, "{what}");
    };
    // The branch only in packed-refs: its file under refs/heads is new after
    // the commit, and packed-refs is not written.
    step("commit on a packed branch", &|| lab.commit("task", "task one"));
    step("commit on a loose branch", &|| lab.commit("task", "task two"));
    step("amend", &|| {
        lab.git("task", &["commit", "--quiet", "--amend", "-m", "task two, amended"]);
    });
    lab.git("repo", &["pack-refs", "--all"]);
    step("reset on a packed branch", &|| {
        lab.git("task", &["reset", "--quiet", "--hard", "HEAD~1"]);
    });
    step("detached checkout of an older commit", &|| {
        lab.git("task", &["checkout", "--quiet", "--detach", "HEAD~1"]);
    });
    step("commit on a detached HEAD", &|| lab.commit("task", "task three"));
}

#[test]
fn a_release_tag_changes_a_declared_file_even_once_packed() {
    let Some(lab) = Lab::new() else { return };
    let step = |what: &str, act: &dyn Fn()| {
        let watched = lab.watched();
        tick();
        let (before, version) = (state(&watched), lab.version());
        act();
        assert_ne!(lab.version(), version, "{what}");
        assert_ne!(state(&watched), before, "{what}");
    };
    step("new tag", &|| {
        lab.git("repo", &["tag", "-a", "v0.2.0", "-m", "v0.2.0"]);
    });
    // The tag only in packed-refs: Git still takes its lock under refs/tags.
    lab.git("repo", &["pack-refs", "--all"]);
    step("packed tag deleted", &|| {
        lab.git("repo", &["tag", "-d", "v0.2.0"]);
    });
}
