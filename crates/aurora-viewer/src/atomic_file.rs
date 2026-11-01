//! Publish complete files atomically; every writer owns its temporary file.
use std::io::Write;
use std::path::Path;

/// Windows refuses a rename onto a file for a moment while another rename
/// replaces it or something (an antivirus, the indexer, a reader opening it)
/// holds it: the same publication a few milliseconds later goes through.
const RENAME_ATTEMPTS: u32 = 6;

/// Publish `bytes` as the file at `path` (counted as the viewer's other
/// files in the disk statistics, `disk_io`), on the disk before it replaces
/// the old file: the user's files (settings, accounts, layout…) survive a
/// hard reset of the PC. Without the flush, ext4 (Linux's default) brought
/// a file created shortly before the reset back empty or not at all: an
/// emptied `login_accounts.json` after a desktop freeze (#1809). Costs a
/// disk flush: off the frame thread.
pub fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    publish_with(path, bytes, true)?;
    crate::disk_io::written(crate::disk_io::Origin::Other, bytes.len());
    Ok(())
}

/// [`write`] without counting the bytes nor the disk flush: for the disk
/// cache, which counts them under its own origin and fetches again what a
/// reset loses.
pub fn publish(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    publish_with(path, bytes, false)
}

fn publish_with(path: &Path, bytes: &[u8], durable: bool) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let temporary = parent.join(format!(".aurora-{}-{}.tmp", std::process::id(), uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
        file.write_all(bytes)?;
        if durable {
            file.sync_data()?;
        }
        drop(file);
        rename_retrying(&temporary, path)?;
        if durable {
            sync_dir(parent);
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// The rename itself on the disk: on Unix a folder's entries are flushed
/// with the folder. NTFS journals the rename, and Windows does not open a
/// folder as a file to flush it. A failure leaves at worst the old file.
#[cfg(unix)]
fn sync_dir(dir: &Path) {
    if let Ok(dir) = std::fs::File::open(dir) {
        let _ = dir.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_: &Path) {}

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
