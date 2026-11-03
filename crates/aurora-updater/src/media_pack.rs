//! The media plugins (SLPlugin, CEF, LibVLC or GStreamer) are published as a
//! pack of their own, once per version of those binaries and not inside every
//! viewer package: they weigh more than the viewer and seldom change. A
//! viewer is built for one pack of its system, named here with its size and
//! SHA-256, and installs it in the user's data folder: nothing downloaded is
//! kept unless it is that exact archive, and a folder only gets its final
//! name once complete.

use crate::{
    Cancel, Failure, Progress,
    feed::Source,
    install::validate_tree,
    package::{Fetch, failure, parse_sha256, unpack},
};
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug)]
pub struct Pack {
    /// A whole number: a higher one replaces the lower ones.
    pub version: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// The pack this viewer uses, built by the "Media plugins" workflow
/// (.github/workflows/media-plugins.yml) and published under the tag
/// `media-plugins-<version>`, one archive per system.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub const PACK: Pack = Pack {
    version: "1",
    size: 252_643_186,
    sha256: "eb2c9c70bcf87df6d987ad38efca83e38a6f2d934fdbc1d00fceb1d20ea1c93e",
};
#[cfg(target_os = "linux")]
pub const PACK: Pack = Pack {
    version: "2",
    size: 206_114_621,
    sha256: "11a41a1e6c6250bbc9875b3fdf2d3609f1dd1740707d9ee805eb245803bea687",
};
#[cfg(target_os = "macos")]
pub const PACK: Pack = Pack {
    version: "3",
    size: 334_919_416,
    sha256: "28b21b12416459b67e7a5148d7a0239830b339db95267b517517b40ba68b15a0",
};

/// The system part of the archive's name.
const SYSTEM: &str = if cfg!(target_os = "linux") {
    "linux-x64"
} else if cfg!(target_os = "macos") {
    "macos-universal"
} else {
    "windows-x64"
};

const MANIFEST: &str = "aurora-media-plugins.json";
/// Written next to the manifest once the archive is extracted: the SHA-256 of
/// that archive. A pack rebuilt under the same version is installed again.
const ORIGIN: &str = "archive.sha256";
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const REQUIRED: [&str; 3] = [
    "llplugin/SLPlugin.exe",
    "llplugin/media_plugin_cef.dll",
    "llplugin/media_plugin_libvlc.dll",
];
/// Video through the system's GStreamer, as in the Legacy Viewer: no
/// LibVLC plugin is built for Linux.
#[cfg(target_os = "linux")]
const REQUIRED: [&str; 3] = [
    "llplugin/SLPlugin",
    "llplugin/libmedia_plugin_cef.so",
    "llplugin/libmedia_plugin_gstreamer.so",
];
/// SLPlugin.app as the Legacy Viewer's application holds it, its plugins
/// in its Contents/Frameworks (DarwinManifest, viewer_manifest.py).
#[cfg(target_os = "macos")]
const REQUIRED: [&str; 3] = [
    "llplugin/SLPlugin.app/Contents/MacOS/SLPlugin",
    "llplugin/SLPlugin.app/Contents/Frameworks/media_plugin_cef.dylib",
    "llplugin/SLPlugin.app/Contents/Frameworks/media_plugin_libvlc.dylib",
];

#[derive(serde::Deserialize)]
struct Manifest {
    schema: u32,
    version: String,
}

impl Pack {
    fn tag(&self) -> String {
        format!("media-plugins-{}", self.version)
    }
    fn folder(&self) -> String {
        format!("Aurora-Media-Plugins-{}-{SYSTEM}", self.version)
    }
    fn complete(&self, dir: &Path) -> bool {
        fs::read(dir.join(MANIFEST))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Manifest>(&bytes).ok())
            .is_some_and(|m| m.schema == 1 && m.version == self.version)
            && fs::read_to_string(dir.join(ORIGIN)).is_ok_and(|hash| hash.trim().eq_ignore_ascii_case(self.sha256))
            && REQUIRED.iter().all(|file| dir.join(file).is_file())
    }
}

/// The folder that holds `llplugin\`, when this pack is installed under `root`.
/// From then on this viewer holds the pack in use until it exits.
pub fn installed(root: &Path, pack: &Pack) -> Option<PathBuf> {
    let held = holds(root, pack.version);
    // locked before the check: a pack being replaced is never found half-way
    if !hold(root, pack.version) {
        return None;
    }
    let dir = root.join(pack.version);
    if pack.complete(&dir) {
        return Some(dir);
    }
    // a damaged pack or another build: kept, it would stop another viewer
    // from repairing a folder this one never uses
    if !held {
        release(root, pack.version);
    }
    None
}

/// Each viewer holds a shared lock on `.in-use-<version>` from the moment it
/// finds that pack until it exits, and a pack is only removed or replaced
/// under an exclusive one. Windows also refuses to rename a folder whose
/// plugins run; Linux and macOS rename and delete it whatever is open in it,
/// so without this lock a newer viewer would take the pack from under an
/// older one still running. The file is never deleted: a viewer may have
/// opened it just before, and would then lock a file no other viewer sees.
fn in_use_path(root: &Path, version: &str) -> PathBuf {
    root.join(format!(".in-use-{version}"))
}

/// The locks this process holds, kept until it exits.
static HELD: std::sync::Mutex<Vec<(PathBuf, File)>> = std::sync::Mutex::new(Vec::new());

fn open_in_use(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)
}

fn holds(root: &Path, version: &str) -> bool {
    let path = in_use_path(root, version);
    HELD.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|(p, _)| *p == path)
}

fn release(root: &Path, version: &str) {
    let path = in_use_path(root, version);
    HELD.lock().unwrap_or_else(|e| e.into_inner()).retain(|(p, _)| *p != path);
}

/// False while another viewer removes or replaces that pack.
fn hold(root: &Path, version: &str) -> bool {
    let path = in_use_path(root, version);
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    if held.iter().any(|(p, _)| *p == path) {
        return true;
    }
    let Ok(file) = open_in_use(&path) else { return false };
    if file.try_lock_shared().is_err() {
        return false;
    }
    held.push((path, file));
    true
}

/// The pack of `version` no longer used by any viewer, this one included:
/// removed or replaced while the returned lock is held.
fn claim(root: &Path, version: &str) -> Option<File> {
    // this viewer is installing another build in its place
    release(root, version);
    let file = open_in_use(&in_use_path(root, version)).ok()?;
    file.try_lock().ok()?;
    Some(file)
}

/// Two viewers started together download the pack once: the second one is
/// told `Busy` and finds the folder later.
fn lock(root: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(".lock"))?;
    file.try_lock().context("another viewer is installing the media plugins")?;
    Ok(file)
}

/// Download and extraction happen here; dropped, it leaves nothing behind.
struct Stage(PathBuf);

impl Stage {
    fn new(root: &Path) -> Result<Self> {
        let path = root.join(format!(".staging-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let path = std::mem::take(&mut self.0);
        // several hundred MB to delete after a failure: not on a runtime thread
        std::thread::spawn(move || {
            if let Err(e) = remove(&path) {
                log::warn!("media plugins staging cleanup: {e:#}");
            }
        });
    }
}

/// Never follows a link or a junction out of the folder.
fn remove(path: &Path) -> Result<()> {
    validate_tree(path)?;
    fs::remove_dir_all(path)?;
    Ok(())
}

/// Called with the lock held, so `stage` is the only staging folder in use:
/// removes what an interrupted viewer left, and the packs of older viewers.
/// A pack still used by a running viewer stays whole.
fn prune(root: &Path, pack: &Pack, stage: &Path) {
    let Ok(current) = pack.version.parse::<u32>() else { return };
    let Ok(entries) = fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if !path.is_dir() || path == stage {
            continue;
        }
        let leftover = name.starts_with(".staging-") || name.starts_with(".trash-");
        let older = name.parse::<u32>().is_ok_and(|v| v < current);
        let target = if older {
            let Some(in_use) = claim(root, &name) else { continue };
            let trash = root.join(format!(".trash-{}", uuid::Uuid::new_v4()));
            if fs::rename(&path, &trash).is_err() {
                continue;
            }
            drop(in_use);
            trash
        } else if leftover {
            path
        } else {
            continue;
        };
        if let Err(e) = remove(&target) {
            log::warn!("media plugins cleanup of {name}: {e:#}");
        }
    }
}

/// Install `pack` under `root` and return the folder that holds `llplugin\`.
pub async fn install(
    source: &Source,
    pack: &Pack,
    root: &Path,
    cancel: &Cancel,
    progress: impl Fn(Progress) + Send + Sync + 'static,
) -> std::result::Result<PathBuf, Failure> {
    cancel.check().map_err(|_| Failure::Network)?;
    let sha256 = parse_sha256(pack.sha256).map_err(|e| failure(Failure::Unsupported, e))?;
    fs::create_dir_all(root).map_err(|e| failure(Failure::Filesystem, e.into()))?;
    let lock = lock(root).map_err(|e| failure(Failure::Busy, e))?;
    if let Some(dir) = installed(root, pack) {
        return Ok(dir);
    }
    let stage = Stage::new(root).map_err(|e| failure(Failure::Filesystem, e))?;
    let folder = pack.folder();
    let url = source
        .asset_url(&pack.tag(), &format!("{folder}.zip"))
        .map_err(|e| failure(Failure::Network, e))?;
    let client = source.client().map_err(|e| failure(Failure::Network, e))?;
    let archive = stage.0.join("download.zip");
    let fetch = Fetch {
        url: &url,
        size: pack.size,
        sha256,
    };
    fetch.to_file(source, &client, &archive, cancel, &progress).await?;
    let (pack, root, cancel) = (*pack, root.to_path_buf(), cancel.clone());
    tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        let prepared = stage.0.join("prepared");
        unpack(&archive, &prepared, &folder, &cancel)?;
        fs::write(prepared.join(ORIGIN), pack.sha256)?;
        ensure!(pack.complete(&prepared), "incomplete media plugins pack");
        let dir = root.join(pack.version);
        if dir.exists() {
            // not this pack (`installed` was checked): another build of the
            // same version, or a damaged folder. Renamed first, so that
            // plugins still running keep every file or lose none.
            let _in_use = claim(&root, pack.version).context("the media plugins to replace are in use")?;
            let trash = root.join(format!(".trash-{}", uuid::Uuid::new_v4()));
            fs::rename(&dir, &trash).context("the media plugins to replace are in use")?;
            if let Err(e) = remove(&trash) {
                log::warn!("media plugins cleanup: {e:#}");
            }
        }
        fs::rename(&prepared, &dir).context("cannot put the media plugins in place")?;
        // held before the install lock is released: no other viewer can
        // claim the pack in between
        if !hold(&root, pack.version) {
            log::warn!("media plugins: pack {} is not marked in use by this viewer", pack.version);
        }
        prune(&root, &pack, &stage.0);
        drop(lock);
        Ok(dir)
    })
    .await
    .map_err(|e| failure(Failure::Package, e.into()))?
    .map_err(|e| failure(Failure::Package, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACK_4: Pack = Pack {
        version: "4",
        size: 0,
        sha256: "",
    };

    fn temp_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("aurora-media-pack-lock-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("root");
        root
    }

    /// Another viewer: a lock of its own on the file, as another process has.
    fn other_viewer(root: &Path, version: &str) -> File {
        let file = open_in_use(&in_use_path(root, version)).expect("lock file");
        file.try_lock_shared().expect("shared lock");
        file
    }

    #[test]
    fn an_older_pack_in_use_stays_until_its_viewer_exits() {
        let root = temp_root();
        let stage = root.join(".staging-current");
        for dir in ["1/llplugin", "3/llplugin", ".staging-current"] {
            fs::create_dir_all(root.join(dir)).expect("fixture");
        }
        let running = other_viewer(&root, "1");
        prune(&root, &PACK_4, &stage);
        assert!(root.join("1/llplugin").is_dir(), "a running viewer keeps its pack");
        assert!(!root.join("3").exists(), "a pack nobody uses is removed");
        assert!(stage.is_dir());
        drop(running);
        prune(&root, &PACK_4, &stage);
        assert!(!root.join("1").exists(), "removed once its viewer has exited");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_pack_is_claimed_only_when_no_viewer_holds_it() {
        let root = temp_root();
        assert!(hold(&root, "4"));
        assert!(hold(&root, "4"), "held once per process");
        let other = open_in_use(&in_use_path(&root, "4")).expect("lock file");
        assert!(other.try_lock().is_err(), "another viewer cannot claim it");
        // this viewer installs another build in its place: its own hold goes
        let claimed = claim(&root, "4").expect("claimed");
        assert!(other.try_lock_shared().is_err(), "nor use it while it is replaced");
        assert!(!hold(&root, "4"));
        drop(claimed);
        assert!(hold(&root, "4"));
        let running = other_viewer(&root, "5");
        assert!(claim(&root, "5").is_none());
        drop(running);
        assert!(claim(&root, "5").is_some());
        drop(other);
        HELD.lock().expect("held").retain(|(p, _)| !p.starts_with(&root));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_damaged_pack_found_at_start_is_not_held() {
        let root = temp_root();
        fs::create_dir_all(root.join("4/llplugin")).expect("damaged pack");
        assert!(installed(&root, &PACK_4).is_none());
        let other = open_in_use(&in_use_path(&root, "4")).expect("lock file");
        assert!(other.try_lock().is_ok(), "another viewer can repair it");
        drop(other);
        let _ = fs::remove_dir_all(&root);
    }
}
