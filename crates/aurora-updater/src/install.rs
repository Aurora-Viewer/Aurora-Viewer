//! A helper outside the installation swaps the package's own entries: the
//! old ones stay recoverable until the new process acknowledges a frame, and
//! whatever else the folder holds is never moved. Windows keeps the running
//! executable mapped, so the helper is a copy run from the transaction
//! folder; Linux and the other Unix systems would not need it for that, but
//! the swap is the same on every system, and the helper also waits for the
//! old viewer to close.

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    process::{Child, Command},
    time::{Duration, Instant},
};

/// The viewer's executable, as the package names it.
pub(crate) const EXE: &str = if cfg!(windows) { "aurora-viewer.exe" } else { "aurora-viewer" };
/// The helper is a copy of the viewer, run from the transaction folder
/// ([`Workspace::handoff`]).
const HELPER: &str = if cfg!(windows) { "helper.exe" } else { "helper" };
const ACK_ENV: &str = "AURORA_UPDATE_ACK";
/// Set on the previous viewer restored after a failed update: the
/// transaction folder, whose `failure.txt` says why ([`failed_update`]).
/// Older helpers set it to `1`.
pub const ROLLBACK_ENV: &str = "AURORA_UPDATE_ROLLBACK";
/// Why the update failed, written by the helper before the rollback.
const FAILURE: &str = "failure.txt";
/// Written by the helper just before it starts the new viewer: a failure
/// without it happened while installing, before any start.
const LAUNCHING: &str = "launching";

/// A failed update, told by its helper to the viewer it restored.
pub struct FailedUpdate {
    /// The helper's own words (`failure.txt`).
    pub reason: String,
    /// The new viewer was being started: it failed to start or to show a
    /// frame. Otherwise the installation itself failed and the previous
    /// one was never stopped from starting.
    pub launched: bool,
}
/// Written by the new viewer once its startup splash is on screen: the
/// helper's own update window can then go without a gap.
const SPLASH_SHOWN: &str = "splash-shown";
/// Written by the new viewer each time its start moves on (a pipeline made,
/// the next step): the helper waits as long as it keeps moving
/// ([`gives_up`]). Older viewers never write it.
const START_PROGRESS: &str = "start-progress";

/// The new viewer is ended when it showed no sign of its start for this
/// long: since its launch while it writes no [`START_PROGRESS`] (a viewer
/// stuck before its renderer, or one older than this rule: the fixed 90 s
/// every helper waited before), since its last step otherwise. A first start
/// compiles every pipeline again (the cache's identity names the build), and
/// slower PCs took more than 90 s for them while the splash counted them:
/// those updates were rolled back (#3072, #3273, #4100 and others).
const START_STALL: Duration = Duration::from_secs(90);
/// However far it moves, a start that has shown no frame after this long is
/// ended.
const START_CEILING: Duration = Duration::from_secs(15 * 60);

/// Whether the helper ends the new viewer, `since_launch` after its launch
/// and `since_progress` after its last step (`None`: none seen).
fn gives_up(since_launch: Duration, since_progress: Option<Duration>) -> bool {
    since_launch >= START_CEILING || since_progress.unwrap_or(since_launch) >= START_STALL
}

/// The step of a [`START_PROGRESS`] file, when it is this transaction's
/// (`expected`: its token and version, one per line).
fn progress_step<'a>(text: &'a str, expected: &str) -> Option<&'a str> {
    text.strip_prefix(expected)?.strip_prefix('\n').filter(|s| !s.is_empty())
}

/// What the helper is doing, for the window the viewer's `main` shows over
/// the whole switch (`splash/`): between the old window's close and the new
/// one's first frame nothing of Aurora was on screen for several seconds
/// (the old process's shutdown, the swap, a cache set aside, the antivirus
/// scan of the new executable).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelperStage {
    /// The previous viewer finishes closing.
    Closing,
    /// The installation folders are swapped.
    Installing,
    /// The cache is set aside (a release that asks for an empty cache).
    ClearingCache,
    /// The new viewer was launched and has not shown anything yet.
    Launching,
    /// The new viewer shows its own splash or window: this one goes.
    Handed,
}
/// The entries of a package folder, the only ones an update moves there.
const PACKAGE: &[&str] = &[
    EXE,
    "assets",
    "LICENSE",
    "NOTICE.md",
    "README.md",
    "version.txt",
    "aurora-install.json",
    // the Linux package's desktop entry and icon (`package.ps1`), named after
    // the viewer's app_id (cli.rs DESKTOP_ID)
    #[cfg(target_os = "linux")]
    "aurora-viewer.desktop",
    #[cfg(target_os = "linux")]
    "aurora-viewer.svg",
];
const MANIFEST: &str = "aurora-install.json";

/// What an update replaces in the folder of the running viewer. Anything
/// else there (a shortcut, notes, a whole Desktop) stays where it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    /// The folder of the release archive: the viewer, its `assets`, its
    /// notices and the manifest only the archive has, so a repository's own
    /// `assets` or `LICENSE` beside a copied viewer are never taken for it.
    Package,
    /// The viewer alone, copied out of its folder (it runs on its own): the
    /// update replaces that file only.
    Executable,
}

impl Layout {
    fn entries(self) -> &'static [&'static str] {
        match self {
            Self::Package => PACKAGE,
            Self::Executable => &[EXE],
        }
    }
}

pub struct Workspace {
    pub root: PathBuf,
    install: PathBuf,
    token: String,
    _lock: File,
    handed_off: bool,
}

fn lock_path(install: &Path) -> Result<PathBuf> {
    Ok(install.parent().context("installation parent")?.join(format!(
        ".{}.update.lock",
        install.file_name().context("installation name")?.to_string_lossy()
    )))
}

fn lock(install: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path(install)?)?;
    file.try_lock().context("another updater owns this installation")?;
    Ok(file)
}

pub(crate) fn plain_dir(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(metadata.is_dir() && !metadata.file_type().is_symlink(), "directory is a link");
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(metadata.file_attributes() & 0x400 == 0, "directory is a reparse point");
    }
    Ok(())
}

/// The installation sits directly at the root of a drive (`E:\Aurora
/// Viewer`, `/aurora`): its parent has no parent, which `validate_install`
/// refuses, so the update's folder, backup and lock are never written at a
/// drive's root.
pub fn at_drive_root(install: &Path) -> bool {
    install.parent().is_some_and(|p| p.parent().is_none())
}

fn validate_install(install: &Path) -> Result<Layout> {
    ensure!(
        install.is_absolute() && install.parent().is_some_and(|p| p.parent().is_some()),
        "unsafe installation root: a drive root or a folder directly at a drive root"
    );
    plain_dir(install)?;
    let exe = fs::symlink_metadata(install.join(EXE)).context("no viewer in this folder")?;
    ensure!(exe.is_file(), "the viewer is a link or not a file");
    let layout = if install.join(MANIFEST).is_file() && install.join("assets").is_dir() && install.join("LICENSE").is_file() {
        Layout::Package
    } else {
        Layout::Executable
    };
    // the archive kept the executable bit and the swap keeps it: a copy of
    // this file is the helper, and the new viewer is started from it
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            fs::metadata(install.join(EXE))?.permissions().mode() & 0o111 != 0,
            "viewer is not executable"
        );
    }
    // Only these entries are ever moved; a link among them, or a junction
    // nested in the assets, must never be followed by the cleanup.
    for name in layout.entries() {
        let path = install.join(name);
        if fs::symlink_metadata(&path).is_ok() {
            plain_entry(&path)?;
        }
    }
    Ok(layout)
}

/// The folder an update was extracted into: exactly a package, nothing else.
fn validate_package(root: &Path) -> Result<()> {
    plain_dir(root)?;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        ensure!(
            PACKAGE.iter().any(|n| entry.file_name().eq_ignore_ascii_case(n)),
            "the update holds a file foreign to the package"
        );
        plain_entry(&entry.path())?;
    }
    ensure!(
        root.join(EXE).is_file() && root.join("assets").is_dir() && root.join("LICENSE").is_file(),
        "incomplete viewer package"
    );
    Ok(())
}

/// A file, or a folder whose whole tree holds no link.
fn plain_entry(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(!metadata.file_type().is_symlink(), "package entry is a link");
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(metadata.file_attributes() & 0x400 == 0, "package entry is a reparse point");
    }
    if metadata.is_dir() {
        validate_tree(path)?;
    }
    Ok(())
}

/// Moves those of `names` that `from` holds into `to`, all or none: after a
/// failure the ones already moved go back. Returns the names moved.
fn move_entries(names: &[&'static str], from: &Path, to: &Path) -> Result<Vec<&'static str>> {
    fs::create_dir_all(to)?;
    let mut moved = Vec::new();
    for &name in names {
        if fs::symlink_metadata(from.join(name)).is_err() {
            continue;
        }
        if let Err(error) = fs::rename(from.join(name), to.join(name)) {
            let error = anyhow::Error::from(error).context(format!("cannot move {name}"));
            for &back in moved.iter().rev() {
                fs::rename(to.join(back), from.join(back)).with_context(|| format!("{error:#}; and {back} could not be put back"))?;
            }
            return Err(error);
        }
        moved.push(name);
    }
    Ok(moved)
}

pub(crate) fn validate_tree(path: &Path) -> Result<()> {
    plain_dir(path)?;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        ensure!(!metadata.file_type().is_symlink(), "package contains a link");
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(metadata.file_attributes() & 0x400 == 0, "package contains a reparse point");
        }
        if metadata.is_dir() {
            validate_tree(&entry.path())?;
        }
    }
    Ok(())
}

/// The refusal a sandbox gives by itself, before the layout and write checks:
/// its folder is a read-only package, so `Protected` must win over the
/// `Unsupported` answer ("extract it into a dedicated folder") that a folder
/// without the release layout would otherwise get. `None` outside a sandbox.
#[cfg(target_os = "linux")]
pub fn sandboxed() -> Option<crate::Failure> {
    let sandbox = sandbox()?;
    log::warn!("update: the viewer runs in a {sandbox} sandbox, which owns its folder");
    Some(crate::Failure::Protected)
}

#[cfg(not(target_os = "linux"))]
pub fn sandboxed() -> Option<crate::Failure> {
    None
}

/// Why this installation cannot replace itself in place, when it cannot: the
/// viewer runs in a sandbox whose folder belongs to a package mounted
/// read-only (Flatpak, Snap, AppImage), or the folder above the installation
/// is not writable by this account (a system package installed by root).
/// `None` when it can. Linux only: Windows has no sandbox of that kind, and
/// its own errors already say that the folder cannot be written.
#[cfg(target_os = "linux")]
pub fn self_update_problem(install: &Path) -> Option<crate::Failure> {
    if let Some(problem) = sandboxed() {
        return Some(problem);
    }
    let parent = install.parent()?;
    // what `Workspace::new` writes a moment later: the staging folder, the
    // lock and the backup all go beside the installation
    let probe = parent.join(format!(".aurora-write-probe-{}", uuid::Uuid::new_v4()));
    match fs::create_dir(&probe) {
        Ok(()) => {
            let _ = fs::remove_dir(&probe);
            None
        }
        Err(error) => {
            log::warn!("update: nothing can be written next to the installation ({error})");
            Some(
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem
                ) {
                    crate::Failure::Protected
                } else {
                    crate::Failure::Filesystem
                },
            )
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub fn self_update_problem(_: &Path) -> Option<crate::Failure> {
    None
}

/// The sandbox the viewer runs in, when it runs in one: its folder is part of
/// a package the sandbox mounts read-only, so the package manager or the
/// bundle replaces the viewer, never an update of ours.
#[cfg(target_os = "linux")]
fn sandbox() -> Option<&'static str> {
    if std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").exists() {
        return Some("Flatpak");
    }
    if std::env::var_os("SNAP").is_some() {
        return Some("Snap");
    }
    if std::env::var_os("APPIMAGE").is_some() || std::env::var_os("APPDIR").is_some() {
        return Some("AppImage");
    }
    None
}

impl Workspace {
    /// Whether the viewer may update itself in place, and why not.
    pub fn check_install(install: &Path) -> Result<()> {
        validate_install(install).map(|_| ())
    }
    pub fn new(install: &Path) -> Result<Self> {
        validate_install(install)?;
        let install = install.canonicalize()?;
        let lock = lock(&install)?;
        sweep_finished(&install);
        let token = uuid::Uuid::new_v4().to_string();
        let root = install
            .parent()
            .context("installation parent")?
            .join(format!(".aurora-update-{token}"));
        fs::create_dir(&root).context("cannot write next to installation")?;
        Ok(Self {
            root,
            install,
            token,
            _lock: lock,
            handed_off: false,
        })
    }

    /// Called on a worker; copies the helper before the viewer shuts down.
    pub fn handoff(mut self, version: &str, maintenance: crate::Maintenance) -> Result<()> {
        let exe = std::env::current_exe()?.canonicalize()?;
        ensure!(exe == self.install.join(EXE), "updater is not running from its installation");
        fs::copy(&exe, self.root.join(HELPER))?;
        let args: Vec<String> = std::env::args_os()
            .skip(1)
            .map(|a| a.into_string().map_err(|_| anyhow::anyhow!("non-Unicode restart argument")))
            .collect::<Result<_>>()?;
        let plan = Plan {
            install: self.install.clone(),
            token: self.token.clone(),
            parent: std::process::id(),
            args,
            version: version.into(),
            maintenance,
        };
        let mut file = OpenOptions::new().write(true).create_new(true).open(self.root.join("plan.json"))?;
        file.write_all(&serde_json::to_vec(&plan)?)?;
        file.sync_all()?;
        drop(file);
        let mut command = Command::new(self.root.join(HELPER));
        command.arg("--aurora-update-helper").arg(&self.root).current_dir(&self.root);
        hidden(&mut command);
        let mut child = command.spawn().context("cannot start update helper")?;
        let start = Instant::now();
        while !self.root.join("helper-ready").exists() {
            if child.try_wait()?.is_some() || start.elapsed() >= Duration::from_secs(15) {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!("update helper did not start");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        self.handed_off = true;
        // Drop the installation lock now: the helper waits for the parent process
        // to exit before taking it and making any change.
        Ok(())
    }
}

use std::io::Write;
impl Drop for Workspace {
    fn drop(&mut self) {
        if !self.handed_off {
            let root = self.root.clone();
            let install = self.install.clone();
            let token = self.token.clone();
            std::thread::spawn(move || {
                if let Err(e) = cleanup_work(&root, &install, &token) {
                    log::warn!("update staging cleanup: {e:#}");
                }
            });
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Plan {
    install: PathBuf,
    token: String,
    parent: u32,
    args: Vec<String>,
    version: String,
    #[serde(default)]
    maintenance: crate::Maintenance,
}

fn validate_work(work: &Path, plan: &Plan) -> Result<()> {
    ensure!(uuid::Uuid::parse_str(&plan.token).is_ok(), "invalid transaction token");
    ensure!(
        work.is_absolute()
            && work.parent() == plan.install.parent()
            && work
                .file_name()
                .is_some_and(|n| n == format!(".aurora-update-{}", plan.token).as_str()),
        "unsafe transaction directory"
    );
    plain_dir(work)?;
    ensure!(
        plan.install.parent().is_some_and(|p| p.parent().is_some()) && plan.install != work,
        "unsafe installation directory"
    );
    Ok(())
}

fn cleanup_work(work: &Path, install: &Path, token: &str) -> Result<()> {
    let plan = Plan {
        install: install.into(),
        token: token.into(),
        parent: 0,
        args: vec![],
        version: String::new(),
        maintenance: Default::default(),
    };
    validate_work(work, &plan)?;
    validate_tree(work)?;
    fs::remove_dir_all(work)?;
    Ok(())
}

/// A finished transaction: failed (`failure.txt`) or committed. Its folder
/// holds a whole copy of the viewer, the failed one included, and nothing
/// resumes it. One whose `backup` is still there is never removed: the
/// restoration failed and that backup is the previous installation.
fn finished(work: &Path) -> bool {
    (work.join(FAILURE).exists() || work.join("committed").exists()) && !work.join("backup").exists()
}

/// The folders earlier updates left beside the installation, removed under
/// the installation lock the caller holds.
fn sweep_finished(install: &Path) {
    let Some(Ok(entries)) = install.parent().map(fs::read_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(token) = name.to_str().and_then(|n| n.strip_prefix(".aurora-update-")) else {
            continue;
        };
        let work = entry.path();
        if uuid::Uuid::parse_str(token).is_ok()
            && finished(&work)
            && let Err(e) = cleanup_work(&work, install, token)
        {
            log::warn!("update: an earlier update's folder is left ({e:#})");
        }
    }
}

/// Why the update that restored this viewer failed, when it did; its
/// transaction folder is then removed in the background, once the helper
/// that restored the viewer has exited. A helper of an older version only
/// says that it failed (`None`).
pub fn failed_update() -> Option<FailedUpdate> {
    let work = PathBuf::from(std::env::var_os(ROLLBACK_ENV)?);
    let result = (|| -> Result<(FailedUpdate, Plan)> {
        let plan: Plan = serde_json::from_slice(&fs::read(work.join("plan.json"))?)?;
        validate_work(&work, &plan)?;
        ensure!(
            std::env::current_exe()?.canonicalize()? == plan.install.join(EXE),
            "rollback notice for another installation"
        );
        let reason = fs::read_to_string(work.join(FAILURE))?;
        let failed = FailedUpdate {
            reason: reason.trim().chars().take(400).collect(),
            launched: work.join(LAUNCHING).exists(),
        };
        Ok((failed, plan))
    })();
    match result {
        Ok((failed, plan)) => {
            let when = if failed.launched { "at its start" } else { "while installing" };
            log::warn!(
                "update: the previous version was restored, the new one failed {when}: {}",
                failed.reason
            );
            std::thread::spawn(move || {
                for _ in 0..120 {
                    if !finished(&work) {
                        return;
                    }
                    if let Ok(_lock) = lock(&plan.install)
                        && cleanup_work(&work, &plan.install, &plan.token).is_ok()
                    {
                        return;
                    }
                    // the helper's own executable is in it until it exits
                    std::thread::sleep(Duration::from_millis(500));
                }
                log::warn!("update: the failed update's folder is left for the next update");
            });
            Some(failed)
        }
        Err(e) => {
            log::warn!("update: the previous version was restored, reason unknown ({e:#})");
            None
        }
    }
}

fn hidden(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

/// The private helper route runs before CLI parsing, logging, GPU or settings.
/// `stage` hears each step of the switch, with the new version.
pub fn run_helper(stage: &mut dyn FnMut(&str, HelperStage)) -> Result<bool> {
    let mut args = std::env::args_os().skip(1);
    if args.next().is_none_or(|a| a != "--aurora-update-helper") {
        return Ok(false);
    }
    let work = PathBuf::from(args.next().context("missing update transaction")?);
    let plan: Plan = serde_json::from_slice(&fs::read(work.join("plan.json"))?)?;
    validate_work(&work, &plan)?;
    ensure!(
        std::env::current_exe()?.canonicalize()? == work.join(HELPER),
        "helper outside transaction"
    );
    let parent = Parent::open(plan.parent)?;
    let version = plan.version.clone();
    let mut stage = |s: HelperStage| stage(&version, s);
    fs::write(work.join("helper-ready"), b"ready")?;
    // after the old viewer's go: its wait for this helper never includes
    // the window's start
    stage(HelperStage::Closing);
    parent.wait()?;
    let result = (|| -> Result<()> {
        stage(HelperStage::Installing);
        let _lock = lock(&plan.install)?;
        validate_install(&plan.install)?;
        validate_package(&work.join("prepared"))?;
        apply(&plan.install, &work, || {
            if plan.maintenance.clear_cache {
                stage(HelperStage::ClearingCache);
            }
            let mut data = crate::user_data::Backup::prepare(&plan.install, &plan.token, plan.maintenance)?;
            stage(HelperStage::Launching);
            let mut command = Command::new(plan.install.join(EXE));
            // a viewer restored by an earlier failure passes its notice on:
            // the new one would open with "the update did not complete"
            command
                .args(&plan.args)
                .env(ACK_ENV, &work)
                .env_remove(ROLLBACK_ENV)
                .current_dir(&plan.install);
            hidden(&mut command);
            fs::write(work.join(LAUNCHING), b"")?;
            let mut child = command.spawn()?;
            let result = await_frame(&mut child, &work, &plan.token, &plan.version, &mut || stage(HelperStage::Handed));
            if !matches!(result, Ok(true)) {
                let _ = child.kill();
                let _ = child.wait();
            }
            if result? {
                data.commit();
                Ok(())
            } else {
                anyhow::bail!("updated viewer did not render a frame")
            }
        })
    })();
    // the new viewer shows, or the old one comes back with its notice
    stage(HelperStage::Handed);
    if let Err(error) = result {
        fs::write(work.join(FAILURE), format!("{error:#}"))?;
        // `apply` restored the directory; launch the old viewer with a visible
        // localized rollback notice. Backups are retained if restoration failed.
        if plan.install.join(EXE).is_file() && !work.join("backup").exists() {
            let mut command = Command::new(plan.install.join(EXE));
            command
                .args(&plan.args)
                .env(ROLLBACK_ENV, &work)
                .env_remove(ACK_ENV)
                .current_dir(&plan.install);
            hidden(&mut command);
            command.spawn()?;
        }
    }
    Ok(true)
}

/// The rollback path is shared with tests using an intentionally failing child.
pub fn apply(install: &Path, work: &Path, launch: impl FnOnce() -> Result<()>) -> Result<()> {
    let install = install.canonicalize()?;
    let install = install.as_path();
    let work = work.canonicalize()?;
    let work = work.as_path();
    let token = work
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix(".aurora-update-"))
        .context("invalid staging name")?;
    validate_work(
        work,
        &Plan {
            install: install.into(),
            token: token.into(),
            parent: 0,
            args: vec![],
            version: String::new(),
            maintenance: Default::default(),
        },
    )?;
    let entries = validate_install(install)?.entries();
    let prepared = work.join("prepared");
    validate_package(&prepared)?;
    let backup = work.join("backup");
    ensure!(!backup.exists(), "backup already exists");
    // `backup` left behind means the previous viewer is in it: only an
    // empty one goes (`remove_dir`), once everything is back in place
    if let Err(error) = move_entries(entries, install, &backup) {
        let _ = fs::remove_dir(&backup);
        return Err(error.context("cannot move old viewer; close other instances"));
    }
    if let Err(error) = move_entries(entries, &prepared, install) {
        move_entries(entries, &backup, install).context("cannot restore old viewer")?;
        let _ = fs::remove_dir(&backup);
        return Err(error);
    }
    match launch() {
        Ok(()) => {
            // A successful new viewer is already running. Cleanup errors cannot
            // turn that into a rollback; leave the backup for later cleanup.
            if let Err(e) = fs::write(work.join("committed"), b"ok") {
                log::warn!("update commit marker: {e}");
            }
            if let Err(e) = validate_tree(&backup).and_then(|()| fs::remove_dir_all(&backup).map_err(Into::into)) {
                log::warn!("update backup cleanup: {e:#}");
            }
            Ok(())
        }
        Err(error) => {
            move_entries(entries, install, &work.join("failed")).context("cannot move failed update")?;
            move_entries(entries, &backup, install).context("cannot restore backup")?;
            if let Err(e) = fs::remove_dir(&backup) {
                log::warn!("update: the emptied backup folder is left ({e})");
            }
            Err(error)
        }
    }
}

/// `handed` runs once the new viewer's splash shows, or its first frame.
/// Waits while the start moves on ([`gives_up`]).
fn await_frame(child: &mut Child, work: &Path, token: &str, version: &str, handed: &mut dyn FnMut()) -> Result<bool> {
    let start = Instant::now();
    let expected = format!("{token}\n{version}");
    let mut splash = false;
    // the last step read, and when it changed
    let mut progress: Option<(String, Instant)> = None;
    loop {
        if child.try_wait()?.is_some() {
            return Ok(false);
        }
        if fs::read_to_string(work.join("first-frame")).is_ok_and(|s| s == expected) {
            if !splash {
                handed();
            }
            return Ok(true);
        }
        if !splash && fs::read_to_string(work.join(SPLASH_SHOWN)).is_ok_and(|s| s == expected) {
            splash = true;
            handed();
        }
        if let Some(text) = fs::read_to_string(work.join(START_PROGRESS)).ok()
            && let Some(step) = progress_step(&text, &expected)
            && progress.as_ref().is_none_or(|(last, _)| last != step)
        {
            progress = Some((step.to_owned(), Instant::now()));
        }
        if gives_up(start.elapsed(), progress.as_ref().map(|(_, at)| at.elapsed())) {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// For the crash watcher of a viewer an update's helper started (it
/// inherits `AURORA_UPDATE_ACK`), once that viewer has ended: whether the
/// helper ended it itself. The helper gives the new version a while to
/// present its first frame, ends it when none came, puts the previous
/// installation back and writes `failure.txt`; the viewer it restores then
/// says that the update did not complete (`failed_update`). That is no
/// crash to report. Waits up to `wait` for `failure.txt` (the restoration
/// comes first); a first frame seen, the update committed or a folder that
/// is not this version's transaction says no. So does a rollback whose
/// restoration failed: `backup` is kept, the previous viewer is not
/// relaunched (`run_helper`'s condition) and no notice shows, so the crash
/// window is the only thing the user sees.
pub fn ended_by_update(wait: Duration) -> bool {
    let Some(path) = std::env::var_os(ACK_ENV) else { return false };
    ended_by_update_at(Path::new(&path), aurora_version::version(), wait)
}

fn ended_by_update_at(work: &Path, version: &str, wait: Duration) -> bool {
    let framed = (|| -> Result<(bool, Plan)> {
        let plan: Plan = serde_json::from_slice(&fs::read(work.join("plan.json"))?)?;
        validate_work(work, &plan)?;
        ensure!(plan.version == version, "another version's update");
        let expected = format!("{}\n{}", plan.token, plan.version);
        Ok((fs::read_to_string(work.join("first-frame")).is_ok_and(|s| s == expected), plan))
    })();
    let plan = match framed {
        Ok((false, plan)) => plan,
        _ => return false,
    };
    let deadline = Instant::now() + wait;
    loop {
        if work.join(FAILURE).exists() {
            // written after `apply` returned, so `backup` is final: the
            // previous version came back with its notice only as `run_helper`
            // relaunches it
            return plan.install.join(EXE).is_file() && !work.join("backup").exists();
        }
        if work.join("committed").exists() || !work.exists() || Instant::now() >= deadline {
            return false;
        }
        // short: the restored viewer, started right after `failure.txt`,
        // must not find the session folder before this watcher removes it
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// This viewer was started by an update's helper, which waits for its first
/// frame.
pub fn started_by_update() -> bool {
    std::env::var_os(ACK_ENV).is_some()
}

/// On the new viewer started by an update: `step` is read every 250 ms on a
/// thread of its own and written for the helper when it changes, until it
/// gives `None` (the first frame is shown). Nothing when the viewer was not
/// started by an update. `step` is any value that changes as the start moves
/// on; the helper only compares it with the last one.
pub fn report_start(step: impl Fn() -> Option<u64> + Send + 'static) {
    let Some(path) = std::env::var_os(ACK_ENV) else { return };
    let spawned = std::thread::Builder::new().name("aurora-update-progress".into()).spawn(move || {
        let opened = (|| -> Result<(PathBuf, Plan)> {
            let work = PathBuf::from(path);
            let plan: Plan = serde_json::from_slice(&fs::read(work.join("plan.json"))?)?;
            validate_work(&work, &plan)?;
            ensure!(plan.version == aurora_version::version(), "update acknowledgement mismatch");
            Ok((work, plan))
        })();
        let (work, plan) = match opened {
            Ok(o) => o,
            Err(e) => {
                log::warn!("update start progress: {e:#}");
                return;
            }
        };
        let mut written = None;
        let mut warned = false;
        while let Some(now) = step() {
            if written != Some(now) {
                // a new file swapped in whole: the helper never reads half
                let part = work.join(format!("{START_PROGRESS}.part"));
                let wrote = fs::write(&part, format!("{}\n{}\n{now}", plan.token, plan.version))
                    .and_then(|()| fs::rename(&part, work.join(START_PROGRESS)));
                match wrote {
                    Ok(()) => written = Some(now),
                    // tried again at the next step; the helper waits 90 s
                    Err(e) if !warned => {
                        warned = true;
                        log::warn!("update start progress: {e}");
                    }
                    Err(_) => {}
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });
    if let Err(e) = spawned {
        log::warn!("update start progress: no thread ({e})");
    }
}

/// The new viewer's startup splash is on screen: the helper's update window
/// goes. Called from the splash's reader thread; nothing when the viewer was
/// not started by an update.
pub fn acknowledge_splash() {
    let Some(path) = std::env::var_os(ACK_ENV) else { return };
    let result = (|| -> Result<()> {
        let work = PathBuf::from(path);
        let plan: Plan = serde_json::from_slice(&fs::read(work.join("plan.json"))?)?;
        validate_work(&work, &plan)?;
        ensure!(plan.version == aurora_version::version(), "update acknowledgement mismatch");
        fs::write(work.join(SPLASH_SHOWN), format!("{}\n{}", plan.token, plan.version))?;
        Ok(())
    })();
    if let Err(e) = result {
        log::warn!("update splash acknowledgement: {e:#}");
    }
}

/// Call only after the render thread reports a presented frame. Disk I/O and
/// waiting for the helper's cleanup stay off the UI thread.
pub fn acknowledge_frame() {
    let Some(path) = std::env::var_os(ACK_ENV) else { return };
    std::thread::spawn(move || {
        let result = (|| -> Result<()> {
            let work = PathBuf::from(path);
            let plan: Plan = serde_json::from_slice(&fs::read(work.join("plan.json"))?)?;
            validate_work(&work, &plan)?;
            ensure!(
                plan.version == aurora_version::version() && std::env::current_exe()?.canonicalize()? == plan.install.join(EXE),
                "update acknowledgement mismatch"
            );
            fs::write(work.join("first-frame"), format!("{}\n{}", plan.token, plan.version))?;
            for _ in 0..300 {
                std::thread::sleep(Duration::from_millis(200));
                if work.join("committed").exists()
                    && let Ok(_lock) = lock(&plan.install)
                    && cleanup_work(&work, &plan.install, &plan.token).is_ok()
                {
                    break;
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            log::warn!("update first-frame acknowledgement: {e:#}");
        }
    });
}

#[cfg(windows)]
struct Parent(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Parent {
    fn open(id: u32) -> Result<Self> {
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE};
        // SAFETY: a PID is passed by our parent; the returned owned handle is
        // used only to wait, never to inspect process memory or credentials.
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, id) };
        ensure!(!handle.is_null(), "cannot wait for parent process");
        Ok(Self(handle))
    }
    fn wait(&self) -> Result<()> {
        use windows_sys::Win32::System::Threading::WaitForSingleObject;
        // SAFETY: the owned handle stays alive throughout the bounded wait.
        ensure!(unsafe { WaitForSingleObject(self.0, 120_000) } == 0, "parent viewer did not exit");
        Ok(())
    }
}
#[cfg(windows)]
impl Drop for Parent {
    fn drop(&mut self) {
        // SAFETY: this is the uniquely owned OpenProcess handle.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
#[cfg(target_os = "linux")]
struct Parent(u32, u64);
#[cfg(target_os = "linux")]
impl Parent {
    /// Opened while the viewer that started the helper is still running (it
    /// waits for `helper-ready`), so the start time read here names it.
    fn open(id: u32) -> Result<Self> {
        let started = process_state(id).context("cannot wait for parent process")?.started;
        Ok(Self(id, started))
    }
    /// The parent exited when its `/proc` entry is gone, or when it is a
    /// zombie the system has not reaped yet. A later process given the same
    /// PID has another start time: the wait does not take it for the parent.
    fn wait(&self) -> Result<()> {
        let start = Instant::now();
        while let Some(state) = process_state(self.0) {
            if state.dead || state.started != self.1 {
                return Ok(());
            }
            ensure!(start.elapsed() < Duration::from_secs(120), "parent viewer did not exit");
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    }
}

/// What `/proc/<pid>/stat` says of a process, for [`Parent`].
#[cfg(target_os = "linux")]
struct ProcessState {
    dead: bool,
    started: u64,
}

#[cfg(target_os = "linux")]
fn process_state(pid: u32) -> Option<ProcessState> {
    parse_stat(&fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// `pid (comm) state ppid … starttime …` (proc_pid_stat(5)): the name may
/// hold spaces and parentheses, so the fields are counted after its last
/// `)`. `state` is field 3, `starttime` field 22. The same rule names a
/// process in `crash::native` (aurora-viewer), which cannot be reached here.
#[cfg(target_os = "linux")]
fn parse_stat(text: &str) -> Option<ProcessState> {
    let mut fields = text.get(text.rfind(')')? + 1..)?.split_ascii_whitespace();
    let state = fields.next()?;
    let started = fields.nth(18)?.parse().ok()?;
    Some(ProcessState {
        dead: matches!(state, "Z" | "X" | "x"),
        started,
    })
}

#[cfg(not(any(windows, target_os = "linux")))]
struct Parent;
#[cfg(not(any(windows, target_os = "linux")))]
impl Parent {
    fn open(_: u32) -> Result<Self> {
        anyhow::bail!("automatic installation is not supported on this system")
    }
    fn wait(&self) -> Result<()> {
        anyhow::bail!("automatic installation is not supported on this system")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_start_that_says_nothing_is_ended_after_90_s_as_before() {
        assert!(!gives_up(Duration::from_secs(89), None));
        assert!(gives_up(Duration::from_secs(90), None));
    }

    #[test]
    fn a_start_that_moves_on_is_waited_for_until_it_stalls_90_s() {
        let min = |m: u64| Duration::from_secs(m * 60);
        // compiling for 5 min, its last pipeline 10 s ago
        assert!(!gives_up(min(5), Some(Duration::from_secs(10))));
        // its last step 89 s ago, then 90
        assert!(!gives_up(min(5), Some(Duration::from_secs(89))));
        assert!(gives_up(min(5), Some(Duration::from_secs(90))));
        // still moving, but no frame after 15 min
        assert!(!gives_up(min(15) - Duration::from_secs(1), Some(Duration::ZERO)));
        assert!(gives_up(min(15), Some(Duration::ZERO)));
    }

    #[test]
    fn only_this_transactions_progress_counts() {
        let expected = "0b6e4b5c-1f6f-4c4e-9a55-2d2f4e8f0a11\n0.9.12";
        assert_eq!(progress_step(&format!("{expected}\n42"), expected), Some("42"));
        // another transaction, another version, a file being written
        assert_eq!(progress_step("ffffffff-1f6f-4c4e-9a55-2d2f4e8f0a11\n0.9.12\n42", expected), None);
        assert_eq!(progress_step(&format!("{expected}0\n42"), expected), None);
        assert_eq!(progress_step(&format!("{expected}\n"), expected), None);
        assert_eq!(progress_step(expected, expected), None);
    }

    #[test]
    fn a_folder_directly_at_a_drive_root_is_told_apart() {
        assert!(at_drive_root(Path::new("/Aurora Viewer")));
        assert!(!at_drive_root(Path::new("/games/Aurora Viewer")));
        assert!(!at_drive_root(Path::new("/")));
        #[cfg(windows)]
        {
            assert!(at_drive_root(Path::new(r"E:\Aurora Viewer")));
            assert!(!at_drive_root(Path::new(r"E:\Games\Aurora Viewer")));
        }
    }

    #[test]
    fn only_finished_updates_without_a_backup_are_swept() {
        let root = std::env::temp_dir().join(format!("aurora-sweep-test-{}", uuid::Uuid::new_v4()));
        let install = root.join("Aurora Viewer");
        fs::create_dir_all(&install).expect("install");
        let work = |marker: &[&str]| {
            let dir = root.join(format!(".aurora-update-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(dir.join("failed")).expect("work");
            fs::write(dir.join("failed").join(EXE), b"new").expect("copy");
            for name in marker {
                if *name == "backup" {
                    fs::create_dir(dir.join(name)).expect("backup");
                } else {
                    fs::write(dir.join(name), b"x").expect("marker");
                }
            }
            dir
        };
        let failed = work(&[FAILURE]);
        let committed = work(&["committed"]);
        let unrestored = work(&[FAILURE, "backup"]);
        let running = work(&["plan.json"]);
        let canonical = install.canonicalize().expect("canonical");
        sweep_finished(&canonical);
        assert!(!failed.exists() && !committed.exists());
        assert!(unrestored.join("backup").exists(), "the previous installation stays");
        assert!(running.exists(), "a transaction in progress stays");
        fs::remove_dir_all(&root).expect("cleanup");
    }

    /// A transaction folder beside an installation, with its plan.
    fn transaction(version: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("aurora-ended-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("root");
        let root = root.canonicalize().expect("canonical root");
        let install = root.join("Aurora Viewer");
        fs::create_dir_all(&install).expect("install");
        fs::write(install.join(EXE), b"previous").expect("restored viewer");
        let token = uuid::Uuid::new_v4().to_string();
        let work = root.join(format!(".aurora-update-{token}"));
        fs::create_dir_all(&work).expect("work");
        let plan = Plan {
            install,
            token,
            parent: 0,
            args: Vec::new(),
            version: version.into(),
            maintenance: Default::default(),
        };
        fs::write(work.join("plan.json"), serde_json::to_vec(&plan).expect("plan")).expect("plan file");
        (root, work)
    }

    #[test]
    fn a_viewer_the_helper_ended_before_its_first_frame_is_no_crash() {
        let wait = Duration::from_millis(50);
        // ended, the update rolled back
        let (root, work) = transaction("1.2.3");
        fs::write(work.join(FAILURE), "updated viewer did not render a frame").expect("failure");
        assert!(ended_by_update_at(&work, "1.2.3", wait));
        // another version's transaction: not ours to say
        assert!(!ended_by_update_at(&work, "1.2.4", wait));
        // the restoration failed: `backup` kept, no viewer relaunched and no
        // notice, so the crash window is reported
        fs::create_dir_all(work.join("backup")).expect("backup");
        assert!(!ended_by_update_at(&work, "1.2.3", wait));
        fs::remove_dir_all(work.join("backup")).expect("no backup");
        // no viewer in the installation: nothing was relaunched either
        let plan: Plan = serde_json::from_slice(&fs::read(work.join("plan.json")).expect("plan")).expect("parse");
        fs::remove_file(plan.install.join(EXE)).expect("no viewer");
        assert!(!ended_by_update_at(&work, "1.2.3", wait));
        fs::write(plan.install.join(EXE), b"previous").expect("viewer back");
        assert!(ended_by_update_at(&work, "1.2.3", wait));
        // a missing or unreadable plan: report
        fs::write(work.join("plan.json"), b"{ not json").expect("bad plan");
        assert!(!ended_by_update_at(&work, "1.2.3", wait));
        fs::remove_file(work.join("plan.json")).expect("no plan");
        assert!(!ended_by_update_at(&work, "1.2.3", wait));
        fs::remove_dir_all(&root).expect("cleanup");
        // a first frame was presented: the helper did not end it
        let (root, work) = transaction("1.2.3");
        let plan: Plan = serde_json::from_slice(&fs::read(work.join("plan.json")).expect("plan")).expect("parse");
        fs::write(work.join("first-frame"), format!("{}\n{}", plan.token, plan.version)).expect("frame");
        fs::write(work.join(FAILURE), "x").expect("failure");
        assert!(!ended_by_update_at(&work, "1.2.3", wait));
        fs::remove_dir_all(&root).expect("cleanup");
        // committed, or nothing said within the wait
        let (root, work) = transaction("1.2.3");
        fs::write(work.join("committed"), "").expect("committed");
        assert!(!ended_by_update_at(&work, "1.2.3", wait));
        fs::remove_file(work.join("committed")).expect("uncommit");
        assert!(!ended_by_update_at(&work, "1.2.3", wait));
        fs::remove_dir_all(&root).expect("cleanup");
        // the transaction folder removed during the wait: report
        let (root, work) = transaction("1.2.3");
        let gone = work.clone();
        let remover = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            fs::remove_dir_all(gone).expect("remove");
        });
        let start = Instant::now();
        assert!(!ended_by_update_at(&work, "1.2.3", Duration::from_secs(5)));
        assert!(start.elapsed() < Duration::from_secs(4), "it waited for the deadline");
        remover.join().expect("remover");
        fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn the_rollback_note_is_waited_for_while_the_installation_is_restored() {
        let (root, work) = transaction("1.2.3");
        let late = work.join(FAILURE);
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            fs::write(late, "updated viewer did not render a frame").expect("failure");
        });
        assert!(ended_by_update_at(&work, "1.2.3", Duration::from_secs(5)));
        writer.join().expect("writer");
        fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn a_folder_at_a_drive_root_is_refused_with_that_reason() {
        let error = validate_install(Path::new("/Aurora Viewer")).expect_err("refused");
        assert!(error.to_string().contains("drive root"), "{error:#}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_folder_this_account_cannot_write_beside_is_refused_as_protected() {
        use std::os::unix::fs::PermissionsExt;
        // inside a package's sandbox the refusal comes first, before any probe
        if sandbox().is_some() {
            return;
        }
        let root = std::env::temp_dir().join(format!("aurora-updater-protected-{}", uuid::Uuid::new_v4()));
        let parent = root.join("parent");
        fs::create_dir_all(&parent).expect("test directory");
        let install = parent.join("install");
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o555)).expect("read-only");
        let writable = fs::write(parent.join("probe"), b"x").is_ok();
        let problem = self_update_problem(&install);
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).expect("writable again");
        if writable {
            // running as root: the permission bits refuse nothing
            let _ = fs::remove_file(parent.join("probe"));
            fs::remove_dir_all(&root).expect("cleanup");
            return;
        }
        assert!(matches!(problem, Some(crate::Failure::Protected)), "{problem:?}");
        assert!(self_update_problem(&install).is_none());
        fs::remove_dir_all(&root).expect("cleanup");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_start_time_is_read_after_the_last_parenthesis_of_the_name() {
        let tail = "S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 987654 20 21";
        let state = parse_stat(&format!("4242 (aurora-viewer) {tail}")).expect("stat");
        assert!(!state.dead);
        assert_eq!(state.started, 987654);
        // a process may name itself with spaces and parentheses
        assert_eq!(parse_stat(&format!("4242 (a) b (c) {tail}")).map(|s| s.started), Some(987654));
        assert!(parse_stat(&format!("4242 (x) Z{}", &tail[1..])).is_some_and(|s| s.dead));
        assert!(parse_stat("4242 (aurora-viewer) S 1 2").is_none());
        assert!(parse_stat("").is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_helper_leaves_as_soon_as_its_parent_is_gone() {
        let mut child = Command::new("sleep").arg("30").spawn().expect("child");
        let parent = Parent::open(child.id()).expect("parent");
        assert!(
            process_state(child.id()).is_some_and(|state| !state.dead),
            "the parent is still running"
        );
        child.kill().expect("kill");
        child.wait().expect("reap");
        let start = Instant::now();
        parent.wait().expect("wait");
        assert!(start.elapsed() < Duration::from_secs(5), "the end was seen at once");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_parent_that_exited_but_was_not_reaped_yet_is_gone_too() {
        let mut child = Command::new("true").spawn().expect("child");
        let parent = Parent::open(child.id()).expect("parent");
        let start = Instant::now();
        // the test is the child's parent: it stays a zombie until reaped here
        while process_state(child.id()).is_some_and(|state| !state.dead) {
            assert!(start.elapsed() < Duration::from_secs(10), "the child never exited");
            std::thread::sleep(Duration::from_millis(10));
        }
        parent.wait().expect("wait");
        child.wait().expect("reap");
    }
}
