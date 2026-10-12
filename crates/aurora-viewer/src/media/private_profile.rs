//! A browser profile that only lives as long as its browser: the web tab of
//! the search window loads an address that carries the session id, and
//! nothing of that page may stay on disk once the browser is gone.
//!
//! The CEF plugin has no in-memory mode: MediaPluginCEF always gives CEF a
//! root cache path, `<cache_path>cef_cache/<plugin pid>` ("set_user_data_path"
//! in indra/media_plugins/cef/media_plugin_cef.cpp), where Chromium keeps its
//! HTTP cache, history, local storage and cookies, and it writes its log to
//! the `cef_log_file` it is given. So such a browser gets a directory of its
//! own under Aurora's cache, holding both, and the directory is **deleted
//! after use**: when the browser is dropped (window closed, logout), again
//! at exit while the plugin lets go of its files, and at the next start for
//! whatever a crash left. While the browser runs its files are on disk.
//!
//! A directory in use holds a lock file locked by its viewer: the sweep at
//! start removes every directory whose lock it can take, and leaves those of
//! another running viewer alone.

use aurora_media::BrowserSettings;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// The folder of the private profiles in Aurora's cache.
pub const ROOT: &str = "web-private";
const LOCK_FILE: &str = "in-use.lock";
/// How long the plugin may take to let go of its files once dropped (it has
/// 14 s to exit before it is killed).
const RELEASE_TIMEOUT: Duration = Duration::from_secs(25);

/// Directories waiting to be removed (their browser is shutting down).
static PENDING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static NEXT: AtomicU32 = AtomicU32::new(0);

pub struct PrivateProfile {
    dir: PathBuf,
    /// Locked while the profile is in use; released before the removal.
    lock: Option<File>,
}

impl PrivateProfile {
    /// A new empty profile under `root`.
    pub fn create(root: &Path) -> std::io::Result<PrivateProfile> {
        let dir = root.join(format!("{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        // what an earlier process of the same id left there
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        std::fs::create_dir_all(&dir)?;
        let lock = File::create(dir.join(LOCK_FILE))?;
        lock.try_lock().map_err(std::io::Error::other)?;
        Ok(PrivateProfile { dir, lock: Some(lock) })
    }

    #[cfg(test)]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// `base` with its cache and its log inside this profile.
    pub fn settings(&self, base: &BrowserSettings) -> BrowserSettings {
        BrowserSettings {
            // the plugin appends "cef_cache" to the path as it is
            cache_path: format!("{}{}", self.dir.display(), std::path::MAIN_SEPARATOR),
            cef_log_file: self.dir.join("cef.log").display().to_string(),
            ..base.clone()
        }
    }
}

impl Drop for PrivateProfile {
    fn drop(&mut self) {
        self.lock = None;
        if let Ok(mut pending) = PENDING.lock() {
            pending.push(self.dir.clone());
        }
        // the plugin still holds its files for a moment: off the frame
        let _ = std::thread::Builder::new()
            .name("aurora-web-profile".into())
            .spawn(|| finish(RELEASE_TIMEOUT));
    }
}

fn removed(dir: &Path) -> bool {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => true,
        Err(e) => e.kind() == std::io::ErrorKind::NotFound || !dir.exists(),
    }
}

/// Remove the profiles whose browser was dropped, trying again until
/// `timeout` while the plugin exits. Called at exit too, once the browsers
/// are gone; what is still held then is removed at the next start. A
/// directory stays listed until it is really gone, so that a caller waiting
/// here does not return while another thread is still removing it.
pub fn finish(timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let waiting: Vec<PathBuf> = match PENDING.lock() {
            Ok(pending) => pending.clone(),
            Err(_) => return,
        };
        let gone: Vec<PathBuf> = waiting.into_iter().filter(|dir| removed(dir)).collect();
        match PENDING.lock() {
            Ok(mut pending) => {
                pending.retain(|dir| !gone.contains(dir));
                if pending.is_empty() {
                    return;
                }
            }
            Err(_) => return,
        }
        if Instant::now() >= deadline {
            log::warn!("web profile: a private browser profile is still in use, it will be removed at the next start");
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// At start: remove every profile under `root` that no running viewer holds
/// (left by a crash or a forced stop).
pub fn sweep(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            let _ = std::fs::remove_file(&dir);
            continue;
        }
        // locked by its viewer while in use
        let in_use = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(dir.join(LOCK_FILE))
            .is_ok_and(|lock| lock.try_lock().is_err());
        if !in_use && !removed(&dir) {
            log::warn!("web profile: could not remove a leftover private browser profile");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aurora-private-profile-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_profile_holds_cache_and_log_and_goes_with_its_browser() {
        let root = root("drop");
        let profile = PrivateProfile::create(&root).expect("profile");
        let dir = profile.dir().to_owned();
        assert!(dir.starts_with(&root) && dir.join(LOCK_FILE).is_file());
        let base = BrowserSettings {
            cache_path: "C:\\shared\\cef\\".into(),
            cef_log_file: "C:\\shared\\cef.log".into(),
            user_agent: "agent".into(),
            ..Default::default()
        };
        let s = profile.settings(&base);
        // both inside the profile, nothing in the shared places
        assert!(Path::new(&s.cache_path).starts_with(&dir));
        assert!(
            s.cache_path.ends_with(std::path::MAIN_SEPARATOR),
            "the plugin appends cef_cache to it"
        );
        assert!(Path::new(&s.cef_log_file).starts_with(&dir));
        assert_eq!((s.user_agent.as_str(), s.cookies_enabled), ("agent", base.cookies_enabled));
        // what the plugin would write
        let cache = dir.join("cef_cache").join("1234").join("Default");
        std::fs::create_dir_all(&cache).expect("cache");
        std::fs::write(cache.join("History"), b"https://search.example/?sid=x").expect("history");
        std::fs::write(dir.join("cef.log"), b"log").expect("log");
        drop(profile);
        finish(Duration::from_secs(10));
        assert!(!dir.exists(), "deleted after use");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_sweep_removes_leftovers_but_not_a_profile_in_use() {
        let root = root("sweep");
        // left by a crashed viewer: files, and a lock nobody holds
        let stale = root.join("999999-0");
        std::fs::create_dir_all(stale.join("cef_cache")).expect("stale");
        std::fs::write(stale.join(LOCK_FILE), b"").expect("lock");
        std::fs::write(stale.join("cef.log"), b"sid=x").expect("log");
        // left without even a lock file
        let bare = root.join("999999-1");
        std::fs::create_dir_all(&bare).expect("bare");
        let live = PrivateProfile::create(&root).expect("live");
        sweep(&root);
        assert!(!stale.exists() && !bare.exists());
        assert!(live.dir().join(LOCK_FILE).is_file(), "a profile in use is left alone");
        let dir = live.dir().to_owned();
        drop(live);
        finish(Duration::from_secs(10));
        assert!(!dir.exists());
        // nothing to do without the folder
        let _ = std::fs::remove_dir_all(&root);
        sweep(&root);
    }
}
