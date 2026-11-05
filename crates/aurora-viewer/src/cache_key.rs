//! The key of the sealed disk cache (`aurora_assets::sealed`): drawn at
//! random on the first start and kept in the system's credential store
//! (Windows Credential Manager, macOS Keychain, Secret Service on Linux),
//! outside the cache folder.
//!
//! When the store is unavailable (Linux without a Secret Service, the update
//! test environment), the key is a file of the configuration folder, apart
//! from the cache all the same. A key lost (store reset, new account) costs
//! the cache only: its files no longer open, so they are cache misses,
//! downloaded and sealed again.

use aurora_assets::sealed::Key;
use std::path::Path;

const SERVICE: &str = "Aurora Viewer cache";
const ACCOUNT: &str = "disk cache key v1";
const FILE: &str = "cache.key";

pub fn load() -> Key {
    if cfg!(test) {
        // unit tests never touch the user's store: one key for the process
        return Key::random().unwrap_or_else(|| Key::from_bytes([0x5a; 32]));
    }
    let started = std::time::Instant::now();
    if aurora_updater::test_environment().is_none()
        && let Some(key) = from_store()
    {
        log::info!("cache key: credential store ({} ms)", started.elapsed().as_millis());
        return key;
    }
    let file = crate::settings::config_dir().join(FILE);
    if let Some(key) = from_file(&file) {
        log::info!("cache key: key file of the configuration folder");
        return key;
    }
    // no store, no file: a key for this session (the cache is lost at exit)
    log::warn!("cache key: none could be kept; this session's cache will not be read again");
    Key::random().unwrap_or_else(|| Key::from_bytes([0x5a; 32]))
}

fn parse(bytes: &[u8]) -> Option<Key> {
    <[u8; 32]>::try_from(bytes).ok().map(Key::from_bytes)
}

fn from_store() -> Option<Key> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|e| log::warn!("cache key: credential store unavailable: {e}"))
        .ok()?;
    match entry.get_secret() {
        Ok(bytes) => match parse(&bytes) {
            Some(key) => return Some(key),
            None => log::warn!("cache key: the stored key is malformed, a new one replaces it"),
        },
        Err(keyring::Error::NoEntry) => {}
        Err(e) => {
            log::warn!("cache key: credential store unreadable: {e}");
            return None;
        }
    }
    let key = Key::random()?;
    if let Err(e) = entry.set_secret(key.as_bytes()) {
        log::warn!("cache key: not kept in the credential store: {e}");
        return None;
    }
    // read back: another viewer starting at the same time may have stored
    // its own key first, and both must seal with the same one
    entry.get_secret().ok().and_then(|b| parse(&b))
}

fn from_file(path: &Path) -> Option<Key> {
    if let Ok(bytes) = std::fs::read(path) {
        if let Some(key) = parse(&bytes) {
            return Some(key);
        }
        log::warn!("cache key: the key file is malformed, a new one replaces it");
    }
    let key = Key::random()?;
    if let Err(e) = write_private(path, key.as_bytes()) {
        log::warn!("cache key: key file not written: {e}");
        return None;
    }
    std::fs::read(path).ok().and_then(|b| parse(&b))
}

/// A file only its owner reads (on Unix; the profile folder already is on
/// Windows).
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    std::io::Write::write_all(&mut options.open(path)?, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_file_is_made_once_then_kept() {
        let dir = std::env::temp_dir().join(format!("aurora-cache-key-{}", uuid::Uuid::new_v4()));
        let path = dir.join(FILE);
        let first = from_file(&path).expect("made");
        assert_eq!(from_file(&path).expect("kept"), first);
        std::fs::write(&path, b"short").expect("damage");
        assert_ne!(from_file(&path).expect("replaced"), first);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
