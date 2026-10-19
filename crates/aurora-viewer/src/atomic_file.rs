//! Publish complete files atomically; every writer owns its temporary file.
use std::io::Write;
use std::path::Path;

/// Windows refuses a rename onto a file for a moment while another rename
/// replaces it or something (an antivirus, the indexer, a reader opening it)
/// holds it: the same publication a few milliseconds later goes through.
const RENAME_ATTEMPTS: u32 = 6;

/// Publish `bytes` as the file at `path` (counted as the viewer's other
/// files in the disk statistics, `disk_io`).
pub fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    publish(path, bytes)?;
    crate::disk_io::written(crate::disk_io::Origin::Other, bytes.len());
    Ok(())
}

/// [`write`] without counting the bytes: for writers that count them
/// under their own origin (the disk cache).
pub fn publish(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let temporary = parent.join(format!(".aurora-{}-{}.tmp", std::process::id(), uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
        file.write_all(bytes)?;
        drop(file);
        rename_retrying(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// `rename`, retried a few times (1 + 2 + 4 + 8 + 16 ms at most) while it is
/// only refused for an access conflict; any other error is returned at once.
fn rename_retrying(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut attempt = 0;
    loop {
        match std::fs::rename(from, to) {
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && attempt + 1 < RENAME_ATTEMPTS => {
                std::thread::sleep(std::time::Duration::from_millis(1 << attempt));
                attempt += 1;
            }
            other => return other,
        }
    }
}

#[cfg(test)]
mod tests {
    /// A reader is never shown a partial file. On Windows, opening the file
    /// in the instant another writer's rename replaces it is refused (access
    /// denied) rather than giving old or new bytes: the reader tries again,
    /// as a cache lookup would on its next request.
    fn read_whole(path: &std::path::Path) -> Vec<u8> {
        let mut attempt = 0;
        loop {
            match std::fs::read(path) {
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && attempt < 50 => {
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                other => return other.expect("read"),
            }
        }
    }

    #[test]
    fn concurrent_writers_publish_whole_files() {
        let root = std::env::temp_dir().join(format!("aurora-atomic-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("directory");
        let target = root.join("asset");
        super::write(&target, &[0; 4096]).expect("initial file");
        std::thread::scope(|scope| {
            for value in 1..8u8 {
                let target = &target;
                scope.spawn(move || {
                    for _ in 0..10 {
                        super::write(target, &[value; 4096]).expect("publish");
                    }
                });
            }
            for _ in 0..80 {
                let bytes = read_whole(&target);
                assert_eq!(bytes.len(), 4096);
                assert!(bytes.iter().all(|v| *v == bytes[0]));
            }
        });
        assert_eq!(std::fs::read_dir(&root).expect("entries").count(), 1);
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
