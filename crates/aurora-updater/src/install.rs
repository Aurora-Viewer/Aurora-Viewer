//! A helper outside the installation performs the directory swap. The old
//! directory remains recoverable until the new process acknowledges a frame.

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    process::{Child, Command},
    time::{Duration, Instant},
};

const EXE: &str = "aurora-viewer.exe";
const ACK_ENV: &str = "AURORA_UPDATE_ACK";
const ALLOWED: &[&str] = &[
    EXE,
    "assets",
    "LICENSE",
    "NOTICE.md",
    "README.md",
    "version.txt",
    "aurora-install.json",
];

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

fn plain_dir(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(metadata.is_dir() && !metadata.file_type().is_symlink(), "directory is a link");
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(metadata.file_attributes() & 0x400 == 0, "directory is a reparse point");
    }
    Ok(())
}

fn validate_install(install: &Path) -> Result<()> {
    ensure!(
        install.is_absolute() && install.parent().is_some_and(|p| p.parent().is_some()),
        "unsafe installation root"
    );
    plain_dir(install)?;
    // Never rename a repository, build tree, or a directory with user files.
    for entry in fs::read_dir(install)? {
        let entry = entry?;
        ensure!(
            ALLOWED.iter().any(|n| entry.file_name().eq_ignore_ascii_case(n)),
            "installation contains non-package files"
        );
        ensure!(!entry.file_type()?.is_symlink(), "package entry is a link");
    }
    ensure!(
        install.join(EXE).is_file() && install.join("assets").is_dir() && install.join("LICENSE").is_file(),
        "viewer is not installed as a portable package"
    );
    // A directory junction nested in the assets must never be followed by cleanup.
    validate_tree(&install.join("assets"))?;
    Ok(())
}

fn validate_tree(path: &Path) -> Result<()> {
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

impl Workspace {
    pub fn supported_install(install: &Path) -> bool {
        validate_install(install).is_ok()
    }
    pub fn new(install: &Path) -> Result<Self> {
        validate_install(install)?;
        let install = install.canonicalize()?;
        let lock = lock(&install)?;
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
    pub fn handoff(mut self, version: &str) -> Result<()> {
        let exe = std::env::current_exe()?.canonicalize()?;
        ensure!(exe == self.install.join(EXE), "updater is not running from its installation");
        fs::copy(&exe, self.root.join("helper.exe"))?;
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
        };
        let mut file = OpenOptions::new().write(true).create_new(true).open(self.root.join("plan.json"))?;
        file.write_all(&serde_json::to_vec(&plan)?)?;
        file.sync_all()?;
        drop(file);
        let mut command = Command::new(self.root.join("helper.exe"));
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
    };
    validate_work(work, &plan)?;
    validate_tree(work)?;
    fs::remove_dir_all(work)?;
    Ok(())
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
pub fn run_helper() -> Result<bool> {
    let mut args = std::env::args_os().skip(1);
    if args.next().is_none_or(|a| a != "--aurora-update-helper") {
        return Ok(false);
    }
    let work = PathBuf::from(args.next().context("missing update transaction")?);
    let plan: Plan = serde_json::from_slice(&fs::read(work.join("plan.json"))?)?;
    validate_work(&work, &plan)?;
    ensure!(
        std::env::current_exe()?.canonicalize()? == work.join("helper.exe"),
        "helper outside transaction"
    );
    let parent = Parent::open(plan.parent)?;
    fs::write(work.join("helper-ready"), b"ready")?;
    parent.wait()?;
    let result = (|| -> Result<()> {
        let _lock = lock(&plan.install)?;
        validate_install(&plan.install)?;
        validate_install(&work.join("prepared"))?;
        apply(&plan.install, &work, || {
            let mut command = Command::new(plan.install.join(EXE));
            command.args(&plan.args).env(ACK_ENV, &work).current_dir(&plan.install);
            hidden(&mut command);
            let mut child = command.spawn()?;
            if await_frame(
                &mut child,
                &work.join("first-frame"),
                &plan.token,
                &plan.version,
                Duration::from_secs(90),
            )? {
                Ok(())
            } else {
                anyhow::bail!("updated viewer did not render a frame")
            }
        })
    })();
    if let Err(error) = result {
        fs::write(work.join("failure.txt"), format!("{error:#}"))?;
        // `apply` restored the directory; launch the old viewer with a visible
        // localized rollback notice. Backups are retained if restoration failed.
        if plan.install.join(EXE).is_file() && !work.join("backup").exists() {
            let mut command = Command::new(plan.install.join(EXE));
            command
                .args(&plan.args)
                .env("AURORA_UPDATE_ROLLBACK", "1")
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
        },
    )?;
    validate_install(install)?;
    validate_install(&work.join("prepared"))?;
    let backup = work.join("backup");
    ensure!(!backup.exists(), "backup already exists");
    fs::rename(install, &backup).context("cannot move old viewer; close other instances")?;
    if let Err(error) = fs::rename(work.join("prepared"), install) {
        fs::rename(&backup, install).context("cannot restore old viewer")?;
        return Err(error.into());
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
            fs::rename(install, work.join("failed")).context("cannot move failed update")?;
            fs::rename(&backup, install).context("cannot restore backup")?;
            Err(error)
        }
    }
}

fn await_frame(child: &mut Child, path: &Path, token: &str, version: &str, timeout: Duration) -> Result<bool> {
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return Ok(false);
        }
        if fs::read_to_string(path).is_ok_and(|s| s == format!("{token}\n{version}")) {
            return Ok(true);
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(100));
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
#[cfg(not(windows))]
struct Parent;
#[cfg(not(windows))]
impl Parent {
    fn open(_: u32) -> Result<Self> {
        anyhow::bail!("automatic installation is Windows-only")
    }
    fn wait(&self) -> Result<()> {
        anyhow::bail!("automatic installation is Windows-only")
    }
}
