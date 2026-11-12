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
    CLIPBOARD_IMAGES,
];

/// The pictures copied from the chat, as PNG files for a paste in the file
/// explorer (`ui/chat/previews/clipboard.rs`): plain files, in the store
/// like the rest so that its size limit and Clear Cache take them too.
pub const CLIPBOARD_IMAGES: &str = "clipboard-images";

/// Bytes written to the cache since the last walk.
static WRITTEN: AtomicU64 = AtomicU64::new(0);

/// The key the cache files are sealed with (cache_key.rs). Read once, on
/// the first use: the app asks for it at startup, before any streamer.
pub fn key() -> &'static aurora_assets::sealed::Key {
    static KEY: std::sync::OnceLock<aurora_assets::sealed::Key> = std::sync::OnceLock::new();
    KEY.get_or_init(crate::cache_key::load)
}

/// Write a cache file, sealed (counted towards the size limit).
pub fn write(path: impl AsRef<Path>, data: &[u8]) -> std::io::Result<()> {
    publish(path.as_ref(), &key().seal(data)?)
}

/// Write a cache file as it is: for what leaves the viewer as a file (the
/// pictures copied from the chat, pasted as files elsewhere).
pub fn write_plain(path: impl AsRef<Path>, data: &[u8]) -> std::io::Result<()> {
    publish(path.as_ref(), data)
}

fn publish(path: &Path, file: &[u8]) -> std::io::Result<()> {
    crate::atomic_file::publish(path, file)?;
    WRITTEN.fetch_add(file.len() as u64, Ordering::Relaxed);
    crate::disk_io::written(origin_of(path), file.len());
    Ok(())
}

/// Whether a file not in the store format reads as it is. Only in the
/// unit tests, which write their files by hand, and in the demo: its
/// sources are plain files (`AURORA_DEMO_STREAM`'s `j2k=` folder, what its
/// local asset server serves, the maps its scenarios write), and it never
/// meets a grid. Anywhere else such a file is a cache miss: the asset is
/// fetched again.
fn plain_files_read() -> bool {
    cfg!(test) || demo()
}

fn demo() -> bool {
    static DEMO: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("AURORA_DEMO").is_some());
    *DEMO
}

/// The stem of the store file of `id`, derived from the installation key
/// ([`aurora_assets::sealed::Names`]): the folders list no asset id. The
/// demo keeps the ids as names: its folder is shared by the demo runs of
/// every build, and the scripts that measure it find its files by id.
pub fn file_name(id: &uuid::Uuid) -> String {
    if demo() { id.to_string() } else { keyed_name(id) }
}

/// [`file_name`] outside the demo, for a read of this installation's store
/// from the demo.
pub fn keyed_name(id: &uuid::Uuid) -> String {
    static NAMES: std::sync::OnceLock<aurora_assets::sealed::Names> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| key().names()).name(id)
}

/// The store file of `id` in `folder` (one of [`DIRS`]) of the store `dir`.
pub fn path(dir: &Path, folder: &str, id: &uuid::Uuid, ext: &str) -> PathBuf {
    dir.join(folder).join(format!("{}.{ext}", file_name(id)))
}

fn open(file: Vec<u8>, plain: bool) -> std::io::Result<Vec<u8>> {
    if plain { key().open_or_plain(file) } else { key().open(file) }
}

/// Read a cache file whole. A file sealed under another key, damaged, or
/// not in the store format is an error, like a missing one: a cache miss.
pub fn read(path: impl AsRef<Path>) -> std::io::Result<Vec<u8>> {
    let d = match std::fs::read(path.as_ref()) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && RENAMES.now(path.as_ref()) => std::fs::read(path.as_ref())?,
        d => d?,
    };
    crate::disk_io::read(origin_of(path.as_ref()), d.len());
    open(d, plain_files_read())
}

/// Read a cache file and mark it as used.
pub fn read_touch(path: impl AsRef<Path>) -> std::io::Result<Vec<u8>> {
    let d = read(path.as_ref())?;
    touch(path.as_ref());
    Ok(d)
}

/// A cache file for reads at given places (only those chunks are decrypted).
pub fn reader(path: impl AsRef<Path>) -> std::io::Result<aurora_assets::sealed::Reader<std::fs::File>> {
    let path = path.as_ref();
    let file = match std::fs::File::open(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && RENAMES.now(path) => std::fs::File::open(path)?,
        file => file?,
    };
    if plain_files_read() {
        aurora_assets::sealed::Reader::new_or_plain(file, key())
    } else {
        aurora_assets::sealed::Reader::new(file, key())
    }
}

/// Delete the cache folders of the earlier format, which the versions
/// before the store left in `old_root` (the cache folder, the store inside it),
/// in the background. They come back if such a version runs again, and go
/// again at the next start.
pub fn remove_unsealed(old_root: PathBuf) {
    let found: Vec<PathBuf> = DIRS.iter().map(|d| old_root.join(d)).filter(|d| d.is_dir()).collect();
    if found.is_empty() {
        return;
    }
    let _ = std::thread::Builder::new().name("aurora-cache-unsealed".into()).spawn(move || {
        let started = Instant::now();
        let mut files = Vec::new();
        for d in &found {
            walk(d, &mut files);
        }
        let mut freed = 0u64;
        for (path, len, _) in &files {
            if std::fs::remove_file(path).is_ok() {
                freed += len;
            }
        }
        for d in &found {
            let _ = std::fs::remove_dir_all(d);
        }
        log::info!(
            "cache: {} MB of the earlier format removed in {:.1} s",
            freed / (1024 * 1024),
            started.elapsed().as_secs_f32()
        );
    });
}

/// The files of a store named by their asset id (the earlier names), waiting
/// to take their name of [`file_name`]: keyed name -> earlier name.
struct Renames(parking_lot::Mutex<Option<std::collections::HashMap<PathBuf, PathBuf>>>);

static RENAMES: Renames = Renames(parking_lot::Mutex::new(None));

/// The folders of [`DIRS`] in renaming order: what a login reads first
/// (the inventory, the region objects), the textures last.
const RENAME_ORDER: [&str; 8] = ["inventory", "objects", "settings", "anim", "sound", "mat", "mesh", "tex"];

impl Renames {
    /// The files of `root` named by an asset id, with their keyed name.
    fn scan(&self, root: &Path) -> Vec<(PathBuf, PathBuf)> {
        let mut found = Vec::new();
        for folder in RENAME_ORDER {
            let dir = root.join(folder);
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some((stem, ext)) = name.to_str().and_then(|n| n.split_once('.')) else {
                    continue;
                };
                // (an id is 36 characters long; a write in progress ends in .tmp)
                if stem.len() != 36 || ext.ends_with("tmp") {
                    continue;
                }
                if let Ok(id) = uuid::Uuid::try_parse(stem) {
                    found.push((dir.join(format!("{}.{ext}", keyed_name(&id))), entry.path()));
                }
            }
        }
        *self.0.lock() = (!found.is_empty()).then(|| found.iter().cloned().collect());
        found
    }

    /// Give the file waiting to be called `keyed` that name now (a read
    /// that comes before its turn). Whether there was one.
    fn now(&self, keyed: &Path) -> bool {
        let Some(earlier) = self.0.lock().as_mut().and_then(|m| m.remove(keyed)) else {
            return false;
        };
        std::fs::rename(&earlier, keyed).is_ok()
    }

    /// Rename every file found by [`Renames::scan`] not renamed yet. A file
    /// already written under its keyed name (fetched again meanwhile) is the
    /// newer one: the earlier file goes. The number renamed.
    fn run(&self, found: Vec<(PathBuf, PathBuf)>) -> usize {
        let mut renamed = 0;
        for (keyed, earlier) in found {
            if self.0.lock().as_mut().and_then(|m| m.remove(&keyed)).is_none() {
                continue;
            }
            if keyed.exists() {
                let _ = std::fs::remove_file(&earlier);
            } else if std::fs::rename(&earlier, &keyed).is_ok() {
                renamed += 1;
            }
        }
        *self.0.lock() = None;
        renamed
    }
}

/// Give the files of the store `root` named by their asset id (written
/// before the names came from the installation key, or by an earlier
/// version since) their keyed names, in the background: no file is fetched
/// again. A rename takes about half a millisecond on Windows (25 s for
/// 50,000 files); a file read before its turn is renamed on the spot. Not
/// in the demo, which keeps the ids as names.
pub fn rename_to_keyed_names(root: PathBuf) {
    let _ = std::thread::Builder::new().name("aurora-cache-names".into()).spawn(move || {
        let started = Instant::now();
        let found = RENAMES.scan(&root);
        if found.is_empty() {
            return;
        }
        let total = found.len();
        let renamed = RENAMES.run(found);
        log::info!(
            "cache: {renamed} of {total} files renamed in {:.1} s",
            started.elapsed().as_secs_f32()
        );
    });
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
    fn a_store_read_takes_only_the_store_format() {
        let plain = [[0xff, 0x4f, 0xff, 0x51].as_slice(), b" a codestream written by hand"].concat();
        assert!(open(plain.clone(), false).is_err(), "a plain file is a miss");
        assert_eq!(open(plain.clone(), true).expect("demo source"), plain);
        let sealed = key().seal(&plain).expect("seal");
        assert_eq!(open(sealed.clone(), false).expect("sealed"), plain);
        assert_eq!(open(sealed, true).expect("sealed"), plain);
    }

    #[test]
    fn store_names_come_from_the_key() {
        let id = uuid::Uuid::from_u128(0xabcd);
        let name = file_name(&id);
        assert_eq!(name, keyed_name(&id));
        assert_eq!(name.len(), aurora_assets::sealed::NAME_LEN);
        assert!(!name.contains(&id.to_string()));
        assert_eq!(
            path(Path::new("store"), "tex", &id, "j2c"),
            Path::new("store").join("tex").join(format!("{name}.j2c"))
        );
    }

    #[test]
    fn files_named_by_id_take_their_keyed_name() {
        let root = std::env::temp_dir().join(format!("aurora-cache-test-{}", uuid::Uuid::new_v4()));
        let (a, b, c) = (uuid::Uuid::from_u128(1), uuid::Uuid::from_u128(2), uuid::Uuid::from_u128(3));
        let earlier = |folder: &str, id: &uuid::Uuid, ext: &str| root.join(folder).join(format!("{id}.{ext}"));
        for folder in DIRS {
            std::fs::create_dir_all(root.join(folder)).expect("folder");
        }
        for (folder, id, ext) in [("tex", a, "j2c"), ("tex", a, "bc7"), ("objects", b, "objc"), ("mesh", c, "mesh")] {
            write(earlier(folder, &id, ext), ext.as_bytes()).expect("earlier file");
        }
        // fetched again before the rename: the newer file stays
        write(path(&root, "mesh", &c, "mesh"), b"newer").expect("newer file");
        let picture = root.join(CLIPBOARD_IMAGES).join("Aurora-image-0123456789abcdef.png");
        write_plain(&picture, b"png").expect("picture");
        let tmp = root.join("tex").join(format!("{a}.tmp"));
        write_plain(&tmp, b"tmp").expect("tmp");

        let renames = Renames(parking_lot::Mutex::new(None));
        let found = renames.scan(&root);
        assert_eq!(found.len(), 4, "the files named by an id, nothing else");
        // a read before its turn
        assert!(renames.now(&path(&root, "objects", &b, "objc")));
        assert!(!renames.now(&path(&root, "objects", &b, "objc")), "once");
        assert_eq!(renames.run(found), 2, "the two textures; the object cache was renamed on read");
        for (folder, id, ext) in [("tex", a, "j2c"), ("tex", a, "bc7"), ("objects", b, "objc")] {
            assert!(!earlier(folder, &id, ext).exists());
            assert_eq!(read(path(&root, folder, &id, ext)).expect("renamed"), ext.as_bytes());
        }
        assert!(!earlier("mesh", &c, "mesh").exists());
        assert_eq!(read(path(&root, "mesh", &c, "mesh")).expect("kept"), b"newer");
        assert!(picture.exists() && tmp.exists(), "left as they are");
        assert!(renames.scan(&root).is_empty(), "nothing left");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn clear_takes_the_copied_pictures() {
        let root = std::env::temp_dir().join(format!("aurora-cache-test-{}", uuid::Uuid::new_v4()));
        let cache = DiskCache::new(root.clone(), 100);
        let picture = root.join(CLIPBOARD_IMAGES).join("Aurora-image-0123456789abcdef.png");
        write_plain(&picture, b"png").expect("picture");
        cache.clear();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while (picture.exists() || cache.is_clearing()) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!picture.exists(), "cleared with the cache");
        let _ = std::fs::remove_dir_all(&root);
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
