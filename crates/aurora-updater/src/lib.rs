//! Portable updates, on Windows and Linux. Optional choices follow
//! llvvmquery.cpp and llvelopack.cpp (the Legacy Viewer, originally LGPL
//! 2.1); package operations use a separate process because Windows keeps
//! the running executable mapped, and because it has to wait for the old
//! viewer to close on every system.
//!
//! The packages differ: a ZIP named after the Windows build, a tar.gz after
//! the Linux one ([`package::Format`]). macOS is installed by hand: its
//! application bundle is signed and replaced as a whole.

pub mod feed;
pub mod ffmpeg_pack;
pub mod install;
pub mod media_pack;
pub mod package;
mod user_data;

pub use user_data::clear_deferred_cache;

/// Effects resolved for the installed version, including skipped releases.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Maintenance {
    pub clear_cache: bool,
    pub reset_settings: bool,
}

pub struct TestEnvironment {
    pub root: std::path::PathBuf,
    pub source: feed::Source,
}

/// A disposable test installation must opt in with a local marker and feed.
/// This also isolates settings/cache and disables credential-store access.
pub fn test_environment() -> Option<&'static TestEnvironment> {
    static TEST: std::sync::OnceLock<Option<TestEnvironment>> = std::sync::OnceLock::new();
    TEST.get_or_init(|| {
        let root = std::path::PathBuf::from(std::env::var_os("AURORA_UPDATE_TEST_ROOT")?);
        if !root.is_absolute() {
            return None;
        }
        let root = root.canonicalize().ok()?;
        let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
        if exe.parent()? != root.join("install") {
            return None;
        }
        let config: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("aurora-update-test.json")).ok()?).ok()?;
        if config["schema"].as_u64()? != 1 {
            return None;
        }
        let source = feed::Source::local(config["feed"].as_str()?).ok()?;
        Some(TestEnvironment { root, source })
    })
    .as_ref()
}

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.0.load(Ordering::Relaxed), "update cancelled");
        Ok(())
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
    pub async fn cancelled(&self) {
        while !self.is_cancelled() {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Failure {
    Network,
    Integrity,
    Filesystem,
    Package,
    Unsupported,
    /// The viewer's folder sits at the root of a drive (`E:\Aurora Viewer`):
    /// no folder above it to hold the update, the backup and the lock.
    DriveRoot,
    /// Something other than the viewer owns the installation folder: a
    /// sandbox's read-only package (Flatpak, Snap, AppImage) or a folder this
    /// account may not write (a system package installed by root). The
    /// package manager or the bundle updates it, never us.
    Protected,
    Busy,
    Startup,
}

/// Whether a release installs itself in place on this system. macOS does
/// not: its application bundle is signed and replaced by hand.
pub const fn installs_here() -> bool {
    cfg!(any(windows, target_os = "linux"))
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Progress {
    pub received: u64,
    pub total: u64,
    pub preparing: bool,
}

/// Build metadata describes provenance, not precedence (SemVer §10).
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (feed::version(latest), feed::version(current)) {
        (Some(a), Some(b)) => a.cmp_precedence(&b).is_gt(),
        _ => false,
    }
}
