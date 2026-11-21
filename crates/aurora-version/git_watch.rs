//! Which Git files the build script declares to Cargo (`rerun-if-changed`),
//! shared by the build script and its integration test.
//!
//! Cargo has no early cut-off: once the script has run again, it rebuilds
//! this crate and every crate that uses it, the viewer included, even when
//! the script printed the same version. So the list holds what can move the
//! version of *this* checkout, and nothing the Git commands of another
//! worktree write.

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git").arg("-C").arg(root).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

/// The version comes from HEAD's commit and the release tags behind it
/// (`git describe`):
///
/// - **HEAD's commit**: `HEAD` (a checkout, a commit on a detached HEAD) and
///   HEAD's reflog, both private to the worktree. Every command that moves a
///   checked-out branch (commit, amend, reset, rebase, merge, pull,
///   cherry-pick) runs in its worktree and writes that reflog; from another
///   worktree, Git refuses to move it (`branch -f`, `fetch`).
/// - **the tags**: `refs/tags`. Git writes a new or moved tag there, and
///   takes the tag's lock there to delete one, even a tag that only
///   `packed-refs` holds. (`refs` when that folder is missing.)
///
/// `packed-refs` and the branch's own file under `refs/heads` are left out:
/// Git rewrites the first when any worktree deletes a packed branch
/// (`branch -D`, `fetch --prune`, `push --delete`) or packs the references,
/// and a pack deletes the second, which Cargo reads as a change. Neither
/// shows a commit on a branch that only `packed-refs` holds: `packed-refs`
/// is not written, and the branch's file, missing until then, could not be
/// declared.
///
/// Without a reflog (`core.logAllRefUpdates` off, or a checkout that never
/// had one) the two files are all there is to go by, and both are declared.
///
/// Not seen: `git update-ref` of this branch from another worktree, which
/// leaves the checkout itself out of step with its branch.
pub fn watched_paths(
    git_dir: &Path,
    common_dir: &Path,
    branch: Option<&str>,
    reflog: bool,
    exists: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    let mut paths = vec![git_dir.join("HEAD")];
    let head_log = git_dir.join("logs").join("HEAD");
    if reflog && exists(&head_log) {
        paths.push(head_log);
    } else {
        paths.extend(branch.map(|branch| common_dir.join(branch)));
        paths.push(common_dir.join("packed-refs"));
    }
    let tags = common_dir.join("refs").join("tags");
    paths.push(if exists(&tags) { tags } else { common_dir.join("refs") });
    // Cargo runs the script at every build for a declared path that is missing.
    paths.retain(|path| exists(path));
    paths
}

/// The list for the checkout that holds `root`; empty without Git.
pub fn watch_list(root: &Path) -> Vec<PathBuf> {
    let absolute = |what: &str| git(root, &["rev-parse", "--path-format=absolute", what]).map(PathBuf::from);
    let (Some(git_dir), Some(common_dir)) = (absolute("--git-dir"), absolute("--git-common-dir")) else {
        return Vec::new();
    };
    let branch = git(root, &["symbolic-ref", "-q", "HEAD"]);
    // Unset means on, in a repository that has a work tree.
    let reflog = git(root, &["config", "--get", "core.logAllRefUpdates"])
        .is_none_or(|value| !matches!(value.to_ascii_lowercase().as_str(), "false" | "no" | "off" | "0"));
    watched_paths(&git_dir, &common_dir, branch.as_deref(), reflog, |path| path.exists())
}
