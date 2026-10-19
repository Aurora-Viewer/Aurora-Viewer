//! The cache folder chosen in the preferences (Legacy Viewer: CacheLocation
//! / NewCacheLocation, LLAppViewer::initCache in llappviewer.cpp, originally
//! LGPL 2.1). Like there, a new folder applies at the next start, and the
//! cache left in the previous folder is then removed (it is not moved).
//!
//! The choice is read from the settings file at startup, before anything
//! touches the cache (`init`, from main), so that every `settings::cache_dir`
//! of the run gives the same folder. The viewer writes in a folder of its own
//! inside the chosen one, so removing the previous cache never touches the
//! user's other files; a marker in the settings folder remembers which cache
//! was used last.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The viewer's folder inside the chosen one.
pub const ROOT_NAME: &str = "AuroraViewerCache";
/// The cache used by the last run (in the settings folder).
const MARKER: &str = "cache_location.txt";

/// The chosen folder of this run (None: the default one).
static CHOSEN: OnceLock<Option<PathBuf>> = OnceLock::new();

/// The cache root for a choice of the settings ("" = the default folder),
/// before the demo's own subfolder.
pub fn root_for(chosen: &str) -> Option<PathBuf> {
    let chosen = chosen.trim();
    (!chosen.is_empty()).then(|| Path::new(chosen).join(ROOT_NAME))
}

/// The chosen root of this run, once `init` ran.
pub fn chosen() -> Option<&'static Path> {
    CHOSEN.get().and_then(|c| c.as_deref())
}

/// Reads the choice from the settings file, checks the folder can be
/// written (else the default folder is used, like initCache's fallback),
/// and removes the cache the previous run used when it was elsewhere.
pub fn init() {
    let path = crate::settings::Settings::path();
    let wanted = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("cache_location").and_then(|c| c.as_str()).map(str::to_owned))
        .unwrap_or_default();
    let chosen = root_for(&wanted).filter(|root| match std::fs::create_dir_all(root) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("cache folder not usable ({e}): the default one is used");
            false
        }
    });
    let _ = CHOSEN.set(chosen);
    let now = crate::settings::cache_dir();
    log::info!("cache folder: {}", now.display());
    let marker = crate::settings::config_dir().join(MARKER);
    let last = std::fs::read_to_string(&marker).ok().map(|s| PathBuf::from(s.trim()));
    if let Some(last) = last.filter(|l| !l.as_os_str().is_empty() && *l != now)
        && is_ours(&last)
    {
        log::info!("cache folder changed: the previous cache is removed");
        let _ = std::thread::Builder::new().name("aurora-cache-purge".into()).spawn(move || {
            if let Err(e) = std::fs::remove_dir_all(&last)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                log::warn!("previous cache not removed: {e}");
            }
        });
    }
    if let Some(dir) = marker.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&marker, now.to_string_lossy().as_bytes()) {
        log::warn!("cache folder marker not written: {e}");
    }
}

/// A cache root the viewer made: the default folder, or its own folder in a
/// chosen one (with the demo's subfolder). Anything else is never removed.
fn is_ours(root: &Path) -> bool {
    let root = if root.file_name().is_some_and(|n| n == "demo") {
        root.parent().unwrap_or(root)
    } else {
        root
    };
    root.file_name().is_some_and(|n| n == ROOT_NAME) || crate::settings::default_cache_dirs().iter().any(|d| d == root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chosen_folder_gets_the_viewers_own_folder_inside() {
        assert_eq!(root_for(""), None);
        assert_eq!(root_for("   "), None);
        assert_eq!(root_for("D:\\Games"), Some(Path::new("D:\\Games").join(ROOT_NAME)));
    }

    #[test]
    fn only_the_viewers_own_folders_are_removed() {
        let mine = Path::new("D:\\Games").join(ROOT_NAME);
        assert!(is_ours(&mine));
        assert!(is_ours(&mine.join("demo")));
        assert!(!is_ours(Path::new("D:\\Games")));
        assert!(!is_ours(Path::new("D:\\Games\\demo")));
        for d in crate::settings::default_cache_dirs() {
            assert!(is_ours(&d));
        }
    }
}
