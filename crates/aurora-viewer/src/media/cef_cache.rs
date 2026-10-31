//! The shared browser cache emptied at every start, like the Legacy Viewer.
//!
//! The CEF plugin gives each of its processes a profile of its own,
//! `<cache_path>cef_cache/<plugin pid>` ("set_user_data_path" in
//! indra/media_plugins/cef/media_plugin_cef.cpp): every prim media, web
//! window or profile page of every session leaves a full Chromium profile
//! there, and no later process reads it again. Left alone they grow without
//! bound (#1714: 92 GB in 770,000 files).
//!
//! Port of LLAppViewer::startCefCachePurge and FSCefCachePurgeThread
//! (indra/newview/llappviewer.cpp): at start, before any browser, the whole
//! `cef_cache` folder is renamed `cef_cache_old`; when that fails (another
//! running viewer holds files in it, or an unfinished purge left the old
//! one), each profile folder is renamed `<pid>_old` instead, and the ones in
//! use stay where they are. A background thread then deletes everything
//! renamed, so a large cache never slows the start. A purge cut short by the
//! exit carries on at the next start.

use std::path::{Path, PathBuf};

/// The folder the plugin appends to the cache path it is given.
pub const DIR: &str = "cef_cache";
const OLD: &str = "_old";

fn is_old(path: &Path) -> bool {
    path.file_name().is_some_and(|n| n.to_string_lossy().ends_with(OLD))
}

fn with_old(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(OLD);
    path.with_file_name(name)
}

/// Rename the profiles of the earlier runs under `root` (the browsers'
/// cache path) so that [`purge`] deletes them. Cheap: renames only, called
/// on the main thread before the first browser starts.
pub fn set_aside(root: &Path) {
    let cache = root.join(DIR);
    if !cache.is_dir() || std::fs::rename(&cache, with_old(&cache)).is_ok() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(&cache) else {
        return;
    };
    for dir in entries.flatten().map(|e| e.path()) {
        // a folder another viewer still uses refuses the rename and stays
        if dir.is_dir() && !is_old(&dir) {
            let _ = std::fs::rename(&dir, with_old(&dir));
        }
    }
}

/// Delete what [`set_aside`] renamed, now or at an earlier start. Slow on a
/// large cache: run off the main thread.
pub fn purge(root: &Path) {
    let cache = root.join(DIR);
    let mut stale: Vec<PathBuf> = vec![with_old(&cache)];
    if let Ok(entries) = std::fs::read_dir(&cache) {
        stale.extend(entries.flatten().map(|e| e.path()).filter(|p| is_old(p)));
    }
    let mut removed = 0usize;
    for dir in stale.iter().filter(|d| d.is_dir()) {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => removed += 1,
            Err(e) => log::warn!("browser cache: could not remove an old profile ({e}), it is tried again at the next start"),
        }
    }
    if removed > 0 {
        log::info!("browser cache: {removed} old profile folder(s) removed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aurora-cef-cache-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn profile(cache: &Path, pid: &str) -> PathBuf {
        let dir = cache.join(pid);
        std::fs::create_dir_all(dir.join("Default").join("Cache")).expect("profile");
        std::fs::write(dir.join("Default").join("Cache").join("data_0"), b"page").expect("file");
        dir
    }

    #[test]
    fn the_earlier_profiles_are_deleted_at_start() {
        let root = root("whole");
        let cache = root.join(DIR);
        profile(&cache, "1234");
        profile(&cache, "5678");
        set_aside(&root);
        assert!(!cache.exists(), "the whole folder is set aside");
        assert!(root.join("cef_cache_old").join("1234").is_dir());
        purge(&root);
        assert!(!root.join("cef_cache_old").exists());
        // what the plugin of this run then creates is left alone
        let live = profile(&cache, "4321");
        purge(&root);
        assert!(live.is_dir());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn without_the_whole_folder_each_profile_is_set_aside() {
        let root = root("each");
        let cache = root.join(DIR);
        // an unfinished purge left the old folder: the rename of the whole
        // folder fails, as it does when another viewer holds files in it
        let unfinished = profile(&root.join("cef_cache_old"), "1111");
        let a = profile(&cache, "1234");
        let left = profile(&cache, "999_old");
        std::fs::write(cache.join("stray.txt"), b"x").expect("file");
        set_aside(&root);
        assert!(!a.exists() && cache.join("1234_old").is_dir());
        assert!(
            left.is_dir() && !cache.join("999_old_old").exists(),
            "an old folder is not renamed again"
        );
        purge(&root);
        assert!(!unfinished.exists() && !root.join("cef_cache_old").exists());
        assert!(!cache.join("1234_old").exists() && !left.exists());
        assert!(cache.join("stray.txt").is_file(), "only profile folders are touched");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn nothing_to_do_without_a_cache() {
        let root = root("none");
        set_aside(&root);
        purge(&root);
        assert!(!root.exists());
    }
}
