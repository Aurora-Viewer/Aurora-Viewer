//! Per-region object cache (the viewer side of `ObjectUpdateCached`, like
//! LLVOCache): compressed object updates keyed by local id with their CRC,
//! persisted to `<dir>/<region id>.objc`. Assets referenced by the objects
//! (textures, meshes...) are cached separately, once, by asset id.
//!
//! The GLTF material overrides of the objects are kept with them (LL's
//! `objects_<x>_<y>_extras.slec`, LLVOCache::writeGenericExtrasToCache): the
//! simulator does not resend them for a cache hit.
//!
//! Reading and writing the file takes milliseconds to tens of milliseconds
//! for a dense region: neither is done by the task that serves the circuits
//! (session/object_cache.rs). `load` and `Snapshot::write` are the blocking
//! halves; `snapshot` is the cheap one the session calls.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;

/// v2 adds the GLTF overrides. A v1 file is dropped: its objects would come
/// back from the cache without their overrides, so the simulator must send
/// them again with full updates (as LL does when the extras file is invalid).
/// Bytes read from and written to the object cache files since the start
/// (the resource monitor's disk statistics).
pub static DISK_READ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static DISK_WRITTEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

const MAGIC: &[u8; 4] = b"AOC2";
/// Entries beyond this are not kept (very dense regions).
const MAX_ENTRIES: usize = 60_000;

/// The payloads are shared, so that a snapshot of a whole region costs one
/// reference count per object and no copy.
#[derive(Clone)]
pub struct Entry {
    pub crc: u32,
    pub update_flags: u32,
    pub data: Arc<[u8]>,
}

pub struct RegionObjectCache {
    path: PathBuf,
    entries: HashMap<u32, Entry>,
    /// Raw override message payload (LLSD notation) by local id.
    overrides: HashMap<u32, Arc<[u8]>>,
    dirty: bool,
    /// Set by a snapshot that could not be written: the next one is taken
    /// even if nothing changed since.
    unwritten: Arc<AtomicBool>,
}

type Decoded = (HashMap<u32, Entry>, HashMap<u32, Arc<[u8]>>);

/// The content of a cache at one moment, to be written by another thread.
pub struct Snapshot {
    path: PathBuf,
    entries: Vec<(u32, Entry)>,
    overrides: Vec<(u32, Arc<[u8]>)>,
    unwritten: Arc<AtomicBool>,
}

impl RegionObjectCache {
    /// A cache with nothing in it, that will be written to the file of this
    /// region.
    pub fn empty(dir: &Path, region_id: Uuid) -> RegionObjectCache {
        RegionObjectCache {
            path: dir.join(format!("{region_id}.objc")),
            entries: HashMap::new(),
            overrides: HashMap::new(),
            dirty: false,
            unwritten: Arc::default(),
        }
    }

    /// Reads the file of a region. Blocking: not for the session loop.
    pub fn load(dir: &Path, region_id: Uuid) -> RegionObjectCache {
        let path = dir.join(format!("{region_id}.objc"));
        let (entries, overrides) = std::fs::read(&path)
            .ok()
            .and_then(|d| {
                DISK_READ.fetch_add(d.len() as u64, std::sync::atomic::Ordering::Relaxed);
                Self::decode(&d)
            })
            .unwrap_or_default();
        if !entries.is_empty() {
            // used: keep it in the rolling cache
            if let Ok(f) = std::fs::File::options().append(true).open(&path) {
                let _ = f.set_modified(std::time::SystemTime::now());
            }
        }
        RegionObjectCache {
            path,
            entries,
            overrides,
            dirty: false,
            unwritten: Arc::default(),
        }
    }

    fn decode(d: &[u8]) -> Option<Decoded> {
        if d.len() < 8 || &d[..4] != MAGIC {
            return None;
        }
        let u32_at = |p: usize| -> Option<u32> { Some(u32::from_le_bytes(d.get(p..p + 4)?.try_into().ok()?)) };
        let count = u32_at(4)? as usize;
        let mut p = 8;
        let mut entries = HashMap::with_capacity(count.min(MAX_ENTRIES));
        for _ in 0..count.min(MAX_ENTRIES) {
            let local_id = u32_at(p)?;
            let crc = u32_at(p + 4)?;
            let update_flags = u32_at(p + 8)?;
            let len = u32_at(p + 12)? as usize;
            let data = d.get(p + 16..p + 16 + len)?.into();
            p += 16 + len;
            entries.insert(local_id, Entry { crc, update_flags, data });
        }
        let count = u32_at(p)? as usize;
        p += 4;
        let mut overrides = HashMap::new();
        for _ in 0..count.min(MAX_ENTRIES) {
            let local_id = u32_at(p)?;
            let len = u32_at(p + 4)? as usize;
            let data = d.get(p + 8..p + 8 + len)?.into();
            p += 8 + len;
            // like readGenericExtrasFromCache: only for cached objects
            if entries.contains_key(&local_id) {
                overrides.insert(local_id, data);
            }
        }
        Some((entries, overrides))
    }

    /// What has to be written, when something changed since the last
    /// snapshot (or when that one could not be written). No copy of the
    /// payloads and no file work: this is what the session loop pays.
    pub fn snapshot(&mut self) -> Option<Snapshot> {
        let retry = self.unwritten.swap(false, Ordering::Relaxed);
        if !self.dirty && !retry {
            return None;
        }
        self.dirty = false;
        Some(Snapshot {
            path: self.path.clone(),
            entries: self.entries.iter().map(|(id, e)| (*id, e.clone())).collect(),
            overrides: self
                .overrides
                .iter()
                .filter(|(id, _)| self.entries.contains_key(id))
                .map(|(id, data)| (*id, data.clone()))
                .collect(),
            unwritten: self.unwritten.clone(),
        })
    }

    /// Cached data if the CRC still matches.
    pub fn get(&self, local_id: u32, crc: u32) -> Option<&Entry> {
        self.entries.get(&local_id).filter(|e| e.crc == crc)
    }

    pub fn put(&mut self, local_id: u32, crc: u32, update_flags: u32, data: &[u8]) {
        if self.entries.len() >= MAX_ENTRIES && !self.entries.contains_key(&local_id) {
            return;
        }
        let same = self.entries.get(&local_id).is_some_and(|e| e.crc == crc && *e.data == *data);
        if !same {
            self.entries.insert(
                local_id,
                Entry {
                    crc,
                    update_flags,
                    data: data.into(),
                },
            );
            self.dirty = true;
        }
    }

    /// Override message payload of an object, replayed on a cache hit.
    pub fn gltf_override(&self, local_id: u32) -> Option<&[u8]> {
        self.overrides.get(&local_id).map(|data| &**data)
    }

    /// Latest override message of an object (each one replaces the previous;
    /// `None` when it cleared them: LLViewerRegion::cacheFullUpdateGLTFOverride).
    pub fn set_gltf_override(&mut self, local_id: u32, payload: Option<&[u8]>) {
        let changed = match payload {
            Some(p) if self.gltf_override(local_id) == Some(p) => false,
            Some(p) => {
                self.overrides.insert(local_id, p.into());
                true
            }
            None => self.overrides.remove(&local_id).is_some(),
        };
        self.dirty |= changed && self.entries.contains_key(&local_id);
    }

    /// Drop every entry without writing (cache cleared on disk).
    pub fn reset(&mut self) {
        self.entries.clear();
        self.overrides.clear();
        self.dirty = false;
        self.unwritten.store(false, Ordering::Relaxed);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Snapshot {
    /// Number of objects.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Writes the file and returns its size. Blocking: not for the session
    /// loop. A failure leaves the previous file and has the cache hand out a
    /// new snapshot the next time it is asked.
    pub fn write(&self) -> std::io::Result<usize> {
        let out = self.encode();
        let written = self.replace_file(&out);
        if written.is_err() {
            self.unwritten.store(true, Ordering::Relaxed);
        }
        written.map(|()| out.len())
    }

    fn encode(&self) -> Vec<u8> {
        let entries: usize = self.entries.iter().map(|(_, e)| e.data.len() + 16).sum();
        let overrides: usize = self.overrides.iter().map(|(_, data)| data.len() + 8).sum();
        let mut out = Vec::with_capacity(12 + entries + overrides);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for (id, e) in &self.entries {
            out.extend_from_slice(&id.to_le_bytes());
            out.extend_from_slice(&e.crc.to_le_bytes());
            out.extend_from_slice(&e.update_flags.to_le_bytes());
            out.extend_from_slice(&(e.data.len() as u32).to_le_bytes());
            out.extend_from_slice(&e.data);
        }
        out.extend_from_slice(&(self.overrides.len() as u32).to_le_bytes());
        for (id, data) in &self.overrides {
            out.extend_from_slice(&id.to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(data);
        }
        out
    }

    fn replace_file(&self, content: &[u8]) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension(format!("{}.tmp", Uuid::new_v4()));
        let result = std::fs::write(&tmp, content).and_then(|()| std::fs::rename(&tmp, &self.path));
        if result.is_ok() {
            DISK_WRITTEN.fetch_add(content.len() as u64, std::sync::atomic::Ordering::Relaxed);
        }
        let _ = std::fs::remove_file(&tmp);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_crc_check() {
        let dir = std::env::temp_dir().join(format!("aurora-objc-{}", Uuid::new_v4()));
        let region = Uuid::new_v4();
        let mut c = RegionObjectCache::load(&dir, region);
        assert!(c.is_empty());
        c.put(42, 7, 1, &[1, 2, 3]);
        c.put(43, 9, 0, &[4]);
        c.set_gltf_override(42, Some(b"{'id':i42}"));
        // not a cached object: not saved
        c.set_gltf_override(99, Some(b"{'id':i99}"));
        let snapshot = c.snapshot().expect("changed");
        assert_eq!(snapshot.len(), 2);
        snapshot.write().expect("written");
        assert!(c.snapshot().is_none(), "nothing changed since");
        let c2 = RegionObjectCache::load(&dir, region);
        assert_eq!(c2.len(), 2);
        assert_eq!(c2.get(42, 7).map(|e| e.data.to_vec()), Some(vec![1, 2, 3]));
        assert!(c2.get(42, 8).is_none(), "stale crc is a miss");
        assert_eq!(c2.gltf_override(42), Some(&b"{'id':i42}"[..]));
        assert_eq!(c2.gltf_override(43), None);
        assert_eq!(c2.gltf_override(99), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_or_old_file_is_ignored() {
        assert!(
            RegionObjectCache::decode(b"AOC2\xff\xff\xff\xff")
                .map(|(m, _)| m.is_empty())
                .unwrap_or(true)
        );
        assert!(RegionObjectCache::decode(b"nope").is_none());
        // v1 (no overrides): dropped so that the simulator resends them
        assert!(RegionObjectCache::decode(b"AOC1\0\0\0\0").is_none());
    }

    #[test]
    fn a_snapshot_holds_what_the_cache_held_when_it_was_taken() {
        let dir = std::env::temp_dir().join(format!("aurora-objc-{}", Uuid::new_v4()));
        let region = Uuid::new_v4();
        let mut c = RegionObjectCache::empty(&dir, region);
        assert!(c.snapshot().is_none(), "an untouched cache is not written");
        c.put(1, 10, 0, &[1]);
        let first = c.snapshot().expect("changed");
        // what arrives while the first one waits for its writer
        c.put(1, 11, 0, &[2]);
        c.put(2, 20, 0, &[3]);
        let second = c.snapshot().expect("changed again");
        first.write().expect("written");
        let read = RegionObjectCache::load(&dir, region);
        assert_eq!(read.len(), 1);
        assert_eq!(read.get(1, 10).map(|e| e.data.to_vec()), Some(vec![1]));
        second.write().expect("written");
        let read = RegionObjectCache::load(&dir, region);
        assert_eq!(read.len(), 2);
        assert_eq!(read.get(1, 11).map(|e| e.data.to_vec()), Some(vec![2]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_snapshot_that_could_not_be_written_is_taken_again() {
        // the folder of the cache is a file: nothing can be written in it
        let blocker = std::env::temp_dir().join(format!("aurora-objc-{}", Uuid::new_v4()));
        std::fs::write(&blocker, b"").expect("file");
        let mut c = RegionObjectCache::empty(&blocker, Uuid::new_v4());
        c.put(1, 10, 0, &[1]);
        assert!(c.snapshot().expect("changed").write().is_err());
        let again = c.snapshot().expect("the failed one is due again");
        assert_eq!(again.len(), 1);
        assert!(again.write().is_err());
        let _ = std::fs::remove_file(&blocker);
    }

    #[test]
    fn concurrent_snapshots_publish_complete_files() {
        let dir = std::env::temp_dir().join(format!("aurora-objc-{}", Uuid::new_v4()));
        let region = Uuid::new_v4();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let writers: Vec<_> = [1_u8, 2]
            .into_iter()
            .map(|value| {
                let dir = dir.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let mut cache = RegionObjectCache::empty(&dir, region);
                    barrier.wait();
                    for round in 0_u32..8 {
                        let revision = u32::from(value) + round * 2;
                        cache.put(42, revision, 0, &vec![revision as u8; 8192]);
                        let snapshot = cache.snapshot().expect("changed");
                        snapshot.write().expect("atomic snapshot");
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().expect("writer completed");
        }
        let cache = RegionObjectCache::load(&dir, region);
        assert_eq!(cache.len(), 1);
        let entry = cache.get(42, 15).or_else(|| cache.get(42, 16)).expect("complete entry");
        assert_eq!(entry.data.len(), 8192);
        assert!(entry.data.iter().all(|byte| u32::from(*byte) == entry.crc));
        assert_eq!(std::fs::read_dir(&dir).expect("cache directory").count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
