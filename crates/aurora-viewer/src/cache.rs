//! Rolling disk cache: textures, meshes, materials, animations, sounds and
//! environment settings (shared by every region, keyed by asset id),
//! per-region object caches, the inventory cache and copied pictures. A background thread
//! keeps the total under the size limit by evicting the least recently used
//! files. The directories are only walked at startup, when the bytes written
//! since push the estimate over the limit, and every 30 minutes (cheap even
//! for a 20 GB cache).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

/// Sub-directories managed by the cache. Every folder written through
/// [`write`] belongs here, or the walks never see it: never trimmed, never
/// counted.
pub const DIRS: [&str; 9] = [
    "tex",
    "mesh",
    "mat",
    "anim",
    "objects",
    "inventory",
    "sound",
    "settings",
    "clipboard-images",
];

/// Bytes written to the cache since the last walk.
static WRITTEN: AtomicU64 = AtomicU64::new(0);

/// Write a cache file (counted towards the size limit).
pub fn write(path: impl AsRef<Path>, data: &[u8]) -> std::io::Result<()> {
    crate::atomic_file::publish(path.as_ref(), data)?;
    WRITTEN.fetch_add(data.len() as u64, Ordering::Relaxed);
    crate::disk_io::written(origin_of(path.as_ref()), data.len());
    Ok(())
}

/// Read a cache file and mark it as used.
pub fn read_touch(path: impl AsRef<Path>) -> std::io::Result<Vec<u8>> {
    let d = std::fs::read(path.as_ref())?;
    crate::disk_io::read(origin_of(path.as_ref()), d.len());
    touch(path.as_ref());
    Ok(d)
}

/// What a cache file holds, by the folder it is in (the disk statistics).
pub fn origin_of(path: &Path) -> crate::disk_io::Origin {
    use crate::disk_io::Origin;
    for part in path.components().rev().skip(1) {
        match part.as_os_str().to_str() {
            Some("tex" | "mat") => return Origin::Textures,
            Some("mesh") => return Origin::Meshes,
            Some("objects") => return Origin::Objects,
            Some("inventory") => return Origin::Inventory,
            _ => {}
        }
    }
    Origin::Other
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
    /// Bytes of each folder of [`DIRS`] at the last walk.
    by_dir: Arc<[AtomicU64; DIRS.len()]>,
    /// The last eviction: when, files removed, bytes freed.
    last_trim: Arc<parking_lot::Mutex<Option<(SystemTime, usize, u64)>>>,
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
        } else if !e.file_name().to_string_lossy().ends_with(".tmp") {
            out.push((e.path(), md.len(), md.modified().unwrap_or(SystemTime::UNIX_EPOCH)));
        }
    }
}

/// Every file of the cache, and the bytes of each folder of [`DIRS`].
fn scan(root: &Path) -> (Vec<(PathBuf, u64, SystemTime)>, [u64; DIRS.len()]) {
    let mut files = Vec::new();
    let mut by_dir = [0u64; DIRS.len()];
    for (i, d) in DIRS.iter().enumerate() {
        let start = files.len();
        walk(&root.join(d), &mut files);
        by_dir[i] = files[start..].iter().map(|f| f.1).sum();
    }
    (files, by_dir)
}

/// The folder of [`DIRS`] a cache file is in.
fn dir_index(root: &Path, path: &Path) -> Option<usize> {
    let first = path.strip_prefix(root).ok()?.components().next()?;
    DIRS.iter().position(|d| first.as_os_str() == *d)
}

fn store_sizes(by_dir: &[AtomicU64; DIRS.len()], sizes: &[u64; DIRS.len()]) {
    for (a, v) in by_dir.iter().zip(sizes) {
        a.store(*v, Ordering::Relaxed);
    }
}

impl DiskCache {
    pub fn new(root: PathBuf, limit_mb: u32) -> DiskCache {
        for d in DIRS {
            let _ = std::fs::create_dir_all(root.join(d));
        }
        let limit = Arc::new(AtomicU64::new(limit_mb as u64 * 1024 * 1024));
        let usage = Arc::new(AtomicU64::new(0));
        let by_dir: Arc<[AtomicU64; DIRS.len()]> = Arc::new(std::array::from_fn(|_| AtomicU64::new(0)));
        let last_trim = Arc::new(parking_lot::Mutex::new(None));
        let clearing = Arc::new(AtomicBool::new(false));
        let (tx, rx) = crossbeam_channel::unbounded::<Cmd>();
        {
            let (root, limit, usage, clearing) = (root.clone(), limit.clone(), usage.clone(), clearing.clone());
            let (by_dir, last_trim) = (by_dir.clone(), last_trim.clone());
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
                            for (path, len, _) in scan(&root).0 {
                                if std::fs::remove_file(&path).is_ok() {
                                    freed += len;
                                }
                            }
                            log::info!("cache cleared ({} MB)", freed / (1024 * 1024));
                            let (files, sizes) = scan(&root);
                            usage.store(files.iter().map(|f| f.1).sum(), Ordering::Relaxed);
                            store_sizes(&by_dir, &sizes);
                            clearing.store(false, Ordering::Release);
                        }
                        Cmd::Trim => {
                            last_walk = Instant::now();
                            WRITTEN.store(0, Ordering::Relaxed);
                            let (mut files, mut sizes) = scan(&root);
                            let mut total: u64 = files.iter().map(|f| f.1).sum();
                            let max = limit.load(Ordering::Relaxed);
                            if total > max {
                                // least recently used first; trim to 85 % to avoid churn
                                files.sort_by_key(|f| f.2);
                                let target = max / 100 * 85;
                                let before = total;
                                let mut removed = 0usize;
                                for (path, len, _) in files {
                                    if total <= target {
                                        break;
                                    }
                                    if std::fs::remove_file(&path).is_ok() {
                                        total = total.saturating_sub(len);
                                        removed += 1;
                                        if let Some(i) = dir_index(&root, &path) {
                                            sizes[i] = sizes[i].saturating_sub(len);
                                        }
                                    }
                                }
                                log::info!("cache trimmed: {removed} files evicted, {} MB used", total / (1024 * 1024));
                                *last_trim.lock() = Some((SystemTime::now(), removed, before - total));
                            }
                            usage.store(total, Ordering::Relaxed);
                            store_sizes(&by_dir, &sizes);
                        }
                    }
                }
            });
        }
        let cache = DiskCache {
            limit,
            usage,
            by_dir,
            last_trim,
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

    /// The size limit (bytes).
    pub fn limit(&self) -> u64 {
        self.limit.load(Ordering::Relaxed)
    }

    /// Bytes of each folder of [`DIRS`] at the last walk (the files
    /// written since are only in [`DiskCache::usage`]).
    pub fn usage_by_dir(&self) -> [u64; DIRS.len()] {
        std::array::from_fn(|i| self.by_dir[i].load(Ordering::Relaxed))
    }

    /// The last eviction of this session: when, files removed, bytes
    /// freed.
    pub fn last_trim(&self) -> Option<(SystemTime, usize, u64)> {
        *self.last_trim.lock()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_count_under_their_folder() {
        use crate::disk_io::Origin;
        let root = Path::new("cache");
        assert_eq!(origin_of(&root.join("tex").join("ab").join("x.j2c")), Origin::Textures);
        assert_eq!(origin_of(&root.join("mesh").join("x")), Origin::Meshes);
        assert_eq!(origin_of(&root.join("inventory").join("x.inv")), Origin::Inventory);
        assert_eq!(origin_of(&root.join("anim").join("x")), Origin::Other);
        assert_eq!(origin_of(&root.join("sound").join("x.ogg")), Origin::Other);
        assert_eq!(origin_of(&root.join("settings").join("x.settings")), Origin::Other);
        // a file named like a folder is not one
        assert_eq!(origin_of(&root.join("tex")), Origin::Other);
    }

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

    #[test]
    fn sounds_and_settings_are_trimmed_and_counted() {
        let root = std::env::temp_dir().join(format!("aurora-cache-test-{}", uuid::Uuid::new_v4()));
        let cache = DiskCache::new(root.clone(), 1);
        // 1210 KB against the 1 MB limit: the oldest file, a sound, goes
        let files = [
            ("sound", "old.ogg", 400),
            ("tex", "a", 400),
            ("tex", "b", 400),
            ("settings", "x.settings", 10),
        ];
        for (i, (dir, name, kb)) in files.iter().enumerate() {
            let p = root.join(dir).join(name);
            std::fs::write(&p, vec![0u8; kb * 1024]).unwrap();
            let t = SystemTime::now() - Duration::from_secs(100 - i as u64 * 10);
            std::fs::File::options().append(true).open(&p).unwrap().set_modified(t).unwrap();
        }
        cache.trim();
        let at = |d: &str| DIRS.iter().position(|x| *x == d).unwrap();
        let want =
            |by_dir: [u64; DIRS.len()]| by_dir[at("sound")] == 0 && by_dir[at("tex")] == 800 * 1024 && by_dir[at("settings")] == 10 * 1024;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !want(cache.usage_by_dir()) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!root.join("sound").join("old.ogg").exists(), "least recently used sound evicted");
        assert!(root.join("tex").join("a").exists() && root.join("tex").join("b").exists());
        assert!(root.join("settings").join("x.settings").exists());
        assert!(want(cache.usage_by_dir()), "counted by folder: {:?}", cache.usage_by_dir());
        assert_eq!(cache.usage(), 810 * 1024);
        let _ = std::fs::remove_dir_all(&root);
    }
}
