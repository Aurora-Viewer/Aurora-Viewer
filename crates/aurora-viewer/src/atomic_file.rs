//! Publish complete files atomically; every writer owns its temporary file.
use std::io::Write;
use std::path::Path;

pub fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let temporary = parent.join(format!(".aurora-{}-{}.tmp", std::process::id(), uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
        file.write_all(bytes)?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
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
                let bytes = std::fs::read(&target).expect("read");
                assert_eq!(bytes.len(), 4096);
                assert!(bytes.iter().all(|v| *v == bytes[0]));
            }
        });
        assert_eq!(std::fs::read_dir(&root).expect("entries").count(), 1);
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
