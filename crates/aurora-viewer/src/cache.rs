//! Rolling disk cache: textures, meshes, materials, animations (shared by
//! every region, keyed by asset id), per-region object caches and the
//! inventory cache. A background thread keeps the total under the size
//! limit by evicting the least recently used files. The directories are
//! only walked at startup, when the bytes written since push the estimate
//! over the limit, and every 30 minutes (cheap even for a 20 GB cache).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

/// Sub-directories managed by the cache.
pub const DIRS: [&str; 6] = ["tex", "mesh", "mat", "anim", "objects", "inventory"];

/// Bytes written to the cache since the last walk.
static WRITTEN: AtomicU64 = AtomicU64::new(0);

/// Write a cache file (counted towards the size limit).
pub fn write(path: impl AsRef<Path>, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, data)?;
    WRITTEN.fetch_add(data.len() as u64, Ordering::Relaxed);
    Ok(())
}

/// Read a cache file and mark it as used.
pub fn read_touch(path: impl AsRef<Path>) -> std::io::Result<Vec<u8>> {
    let d = std::fs::read(path.as_ref())?;
    touch(path.as_ref());
    Ok(d)
}

/// Mark a cache file as recently used (LRU eviction works on mtime).
pub fn touch(path: &Path) {
    if let Ok(f) = std::fs::File::options().append(true).open(path) {
        let _ = f.set_modified(SystemTime::now());
    }
}

enum Cmd {
    Trim,
    Clear,
}

pub struct DiskCache {
    limit: Arc<AtomicU64>,
    usage: Arc<AtomicU64>,
    clearing: Arc<AtomicBool>,
    tx: crossbeam_channel::Sender<Cmd>,
}

fn walk(dir: &Path, out: &mut Vec<(PathBuf, u64, SystemTime)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let Ok(md) = e.metadata() else {
            continue;
        };
        if md.is_dir() {
            walk(&e.path(), out);
        } else {
            out.push((e.path(), md.len(), md.modified().unwrap_or(SystemTime::UNIX_EPOCH)));
        }
    }
}

fn scan(root: &Path) -> Vec<(PathBuf, u64, SystemTime)> {
    let mut files = Vec::new();
    for d in DIRS {
        walk(&root.join(d), &mut files);
    }
    files
}

impl DiskCache {
    pub fn new(root: PathBuf, limit_mb: u32) -> DiskCache {
        for d in DIRS {
            let _ = std::fs::create_dir_all(root.join(d));
        }
        let limit = Arc::new(AtomicU64::new(limit_mb as u64 * 1024 * 1024));
        let usage = Arc::new(AtomicU64::new(0));
        let clearing = Arc::new(AtomicBool::new(false));
        let (tx, rx) = crossbeam_channel::unbounded::<Cmd>();
        {
            let (root, limit, usage, clearing) = (root.clone(), limit.clone(), usage.clone(), clearing.clone());
            let mut last_walk = Instant::now();
            let _ = std::thread::Builder::new().name("aurora-cache".into()).spawn(move || {
                loop {
                    let cmd = match rx.recv_timeout(Duration::from_secs(30)) {
                        // coalesce queued trims (e.g. while the size slider moves)
                        Ok(Cmd::Trim) => {
                            std::thread::sleep(Duration::from_millis(300));
                            let mut clear = false;
                            while let Ok(c) = rx.try_recv() {
                                clear |= matches!(c, Cmd::Clear);
                            }
                            if clear { Cmd::Clear } else { Cmd::Trim }
                        }
                        Ok(c) => c,
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                            // estimate from the bytes written; walk only when needed
                            let estimate = usage.load(Ordering::Relaxed) + WRITTEN.swap(0, Ordering::Relaxed);
                            usage.store(estimate, Ordering::Relaxed);
                            if estimate <= limit.load(Ordering::Relaxed) && last_walk.elapsed() < Duration::from_secs(1800) {
                                continue;
                            }
                            Cmd::Trim
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
                    };
                    match cmd {
                        Cmd::Clear => {
                            let mut freed = 0u64;
                            for (path, len, _) in scan(&root) {
                                if std::fs::remove_file(&path).is_ok() {
                                    freed += len;
                                }
                            }
                            log::info!("cache cleared ({} MB)", freed / (1024 * 1024));
                            usage.store(scan(&root).iter().map(|f| f.1).sum(), Ordering::Relaxed);
                            clearing.store(false, Ordering::Release);
                        }
                        Cmd::Trim => {
                            last_walk = Instant::now();
                            WRITTEN.store(0, Ordering::Relaxed);
                            let mut files = scan(&root);
                            let mut total: u64 = files.iter().map(|f| f.1).sum();
                            let max = limit.load(Ordering::Relaxed);
                            if total > max {
                                // least recently used first; trim to 85 % to avoid churn
                                files.sort_by_key(|f| f.2);
                                let target = max / 100 * 85;
                                let mut removed = 0usize;
                                for (path, len, _) in files {
                                    if total <= target {
                                        break;
                                    }
                                    if std::fs::remove_file(&path).is_ok() {
                                        total = total.saturating_sub(len);
                                        removed += 1;
                                    }
                                }
                                log::info!("cache trimmed: {removed} files evicted, {} MB used", total / (1024 * 1024));
                            }
                            usage.store(total, Ordering::Relaxed);
                        }
                    }
                }
            });
        }
        let cache = DiskCache {
            limit,
            usage,
            clearing,
            tx,
        };
        cache.trim();
        cache
    }

    pub fn set_limit_mb(&self, mb: u32) {
        let new = mb as u64 * 1024 * 1024;
        if self.limit.swap(new, Ordering::Relaxed) != new {
            self.trim();
        }
    }

    /// Re-measure and evict if over the limit (runs in the background).
    pub fn trim(&self) {
        let _ = self.tx.send(Cmd::Trim);
    }

    /// Delete every cached file (in the background).
    pub fn clear(&self) {
        self.clearing.store(true, Ordering::Release);
        let _ = self.tx.send(Cmd::Clear);
    }

    pub fn is_clearing(&self) -> bool {
        self.clearing.load(Ordering::Acquire)
    }

    /// Last measured size in bytes.
    pub fn usage(&self) -> u64 {
        self.usage.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_least_recently_used() {
        let root = std::env::temp_dir().join(format!("aurora-cache-test-{}", uuid::Uuid::new_v4()));
        let cache = DiskCache::new(root.clone(), 1);
        // 3 files of 400 KB: over the 1 MB limit
        for (i, name) in ["a", "b", "c"].iter().enumerate() {
            let p = root.join("tex").join(name);
            std::fs::write(&p, vec![0u8; 400 * 1024]).unwrap();
            let t = SystemTime::now() - Duration::from_secs(100 - i as u64 * 10);
            std::fs::File::options().append(true).open(&p).unwrap().set_modified(t).unwrap();
        }
        // "a" is the oldest but was just used
        touch(&root.join("tex").join("a"));
        cache.trim();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while root.join("tex").join("b").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!root.join("tex").join("b").exists(), "least recently used evicted");
        assert!(root.join("tex").join("a").exists(), "touched file kept");
        let _ = std::fs::remove_dir_all(&root);
    }
}
