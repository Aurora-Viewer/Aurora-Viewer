//! The files demo scenarios take from a folder: AURORA_DEMO_ARRIVAL's
//! `<folder>`, AURORA_DEMO_STREAM's `j2k=`, and the interface sounds of
//! AURORA_DEMO_SOUND. Plain asset files are read as they are in every build.
//! Files of the disk cache's format (cache.rs, sealed with this
//! installation's key) are read through `crate::cache` by development
//! builds only; a build of the published profile (`release`) leaves them
//! out, so its demo scenarios read plain folders and the demo's own files.

use std::io::Read;
use std::path::Path;

/// Whether this build reads files of the cache's format for the scenarios.
pub fn reads_cache_files() -> bool {
    reads_cache_files_in(env!("AURORA_BUILD_PROFILE"))
}

/// Every profile but the published one (`dev`, `ci`, `debugging`).
fn reads_cache_files_in(profile: &str) -> bool {
    profile != "release"
}

/// A file of a scenario's folder, whole: as it is when plain, opened
/// through `crate::cache` when it is in the cache's format and the build
/// reads those.
pub fn read(path: &Path) -> std::io::Result<Vec<u8>> {
    read_in(path, reads_cache_files())
}

fn read_in(path: &Path, cache_files: bool) -> std::io::Result<Vec<u8>> {
    if !in_cache_format(path)? {
        return std::fs::read(path);
    }
    if !cache_files {
        tell_plain_files_only();
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "a file of the cache's format: development builds only",
        ));
    }
    crate::cache::read(path)
}

/// Whether a file of a scenario's folder can be read (a published build
/// opens its head only): to leave the others out of the lists the
/// scenarios cycle.
pub fn usable(path: &Path) -> bool {
    usable_in(path, reads_cache_files())
}

fn usable_in(path: &Path, cache_files: bool) -> bool {
    // (a development build reads every file: no head to open)
    if cache_files {
        return path.is_file();
    }
    match in_cache_format(path) {
        Ok(false) => true,
        Ok(true) => {
            tell_plain_files_only();
            false
        }
        Err(_) => false,
    }
}

/// The file starts with the header of the cache's format.
fn in_cache_format(path: &Path) -> std::io::Result<bool> {
    let mut head = Vec::with_capacity(aurora_assets::sealed::HEADER);
    std::fs::File::open(path)?
        .take(aurora_assets::sealed::HEADER as u64)
        .read_to_end(&mut head)?;
    Ok(aurora_assets::sealed::is_sealed(&head))
}

/// Once a session, in the log: why the scenario's folder looks empty.
fn tell_plain_files_only() {
    static TOLD: std::sync::Once = std::sync::Once::new();
    TOLD.call_once(|| {
        log::warn!("demo: files of the cache's format are read by development builds only; this build takes folders of plain asset files");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_published_profile_alone_takes_plain_files_only() {
        assert!(!reads_cache_files_in("release"));
        for profile in ["debug", "ci", "debugging"] {
            assert!(reads_cache_files_in(profile), "{profile}");
        }
    }

    #[test]
    fn plain_files_read_in_every_build_cache_files_in_development_ones() {
        let dir = std::env::temp_dir().join(format!("aurora-demo-source-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("folder");
        let plain = dir.join("plain.j2c");
        std::fs::write(&plain, b"\xff\x4f\xff\x51 a codestream").expect("plain file");
        let cached = dir.join("cached.j2c");
        crate::cache::write(&cached, b"\xff\x4f\xff\x51 a cached codestream").expect("cache file");
        let short = dir.join("short.j2c");
        std::fs::write(&short, b"AUSL").expect("short file");

        for cache_files in [false, true] {
            assert_eq!(read_in(&plain, cache_files).expect("plain"), b"\xff\x4f\xff\x51 a codestream");
            assert!(usable_in(&plain, cache_files));
            // shorter than the cache's header: a plain file
            assert_eq!(read_in(&short, cache_files).expect("short"), b"AUSL");
            assert!(read_in(&dir.join("missing.j2c"), cache_files).is_err());
            assert!(!usable_in(&dir.join("missing.j2c"), cache_files));
        }
        assert_eq!(read_in(&cached, true).expect("cache file"), b"\xff\x4f\xff\x51 a cached codestream");
        assert!(usable_in(&cached, true));
        assert!(read_in(&cached, false).is_err());
        assert!(!usable_in(&cached, false));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
