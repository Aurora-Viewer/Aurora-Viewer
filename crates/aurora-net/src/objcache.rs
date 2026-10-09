//! Per-region object cache (the viewer side of `ObjectUpdateCached`, like
//! LLVOCache): compressed object updates keyed by local id with their CRC,
//! persisted to `<dir>/<region id>.objc`. Assets referenced by the objects
//! (textures, meshes...) are cached separately, once, by asset id.
//!
//! The GLTF material overrides of the objects are kept with them (LL's
//! `objects_<x>_<y>_extras.slec`, LLVOCache::writeGenericExtrasToCache): the
//! simulator does not resend them for a cache hit.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// v2 adds the GLTF overrides. A v1 file is dropped: its objects would come
/// back from the cache without their overrides, so the simulator must send
/// them again with full updates (as LL does when the extras file is invalid).
const MAGIC: &[u8; 4] = b"AOC2";
/// Entries beyond this are not kept (very dense regions).
const MAX_ENTRIES: usize = 60_000;

pub struct Entry {
    pub crc: u32,
    pub update_flags: u32,
    pub data: Vec<u8>,
}

pub struct RegionObjectCache {
    path: PathBuf,
    entries: HashMap<u32, Entry>,
    /// Raw override message payload (LLSD notation) by local id.
    overrides: HashMap<u32, Vec<u8>>,
    dirty: bool,
}

type Decoded = (HashMap<u32, Entry>, HashMap<u32, Vec<u8>>);

impl RegionObjectCache {
    pub fn load(dir: &Path, region_id: Uuid) -> RegionObjectCache {
        let path = dir.join(format!("{region_id}.objc"));
        let (entries, overrides) = std::fs::read(&path).ok().and_then(|d| Self::decode(&d)).unwrap_or_default();
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
            let data = d.get(p + 16..p + 16 + len)?.to_vec();
            p += 16 + len;
            entries.insert(local_id, Entry { crc, update_flags, data });
        }
        let count = u32_at(p)? as usize;
        p += 4;
        let mut overrides = HashMap::new();
        for _ in 0..count.min(MAX_ENTRIES) {
            let local_id = u32_at(p)?;
            let len = u32_at(p + 4)? as usize;
            let data = d.get(p + 8..p + 8 + len)?.to_vec();
            p += 8 + len;
            // like readGenericExtrasFromCache: only for cached objects
            if entries.contains_key(&local_id) {
                overrides.insert(local_id, data);
            }
        }
        Some((entries, overrides))
    }

    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        let mut out = Vec::with_capacity(8 + self.entries.values().map(|e| e.data.len() + 16).sum::<usize>());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for (id, e) in &self.entries {
            out.extend_from_slice(&id.to_le_bytes());
            out.extend_from_slice(&e.crc.to_le_bytes());
            out.extend_from_slice(&e.update_flags.to_le_bytes());
            out.extend_from_slice(&(e.data.len() as u32).to_le_bytes());
            out.extend_from_slice(&e.data);
        }
        let overrides: Vec<_> = self.overrides.iter().filter(|(id, _)| self.entries.contains_key(id)).collect();
        out.extend_from_slice(&(overrides.len() as u32).to_le_bytes());
        for (id, data) in overrides {
            out.extend_from_slice(&id.to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(data);
        }
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self.path.with_extension("tmp");
        if std::fs::write(&tmp, &out).is_ok() && std::fs::rename(&tmp, &self.path).is_ok() {
            self.dirty = false;
        }
    }

    /// Cached data if the CRC still matches.
    pub fn get(&self, local_id: u32, crc: u32) -> Option<&Entry> {
        self.entries.get(&local_id).filter(|e| e.crc == crc)
    }

    pub fn put(&mut self, local_id: u32, crc: u32, update_flags: u32, data: &[u8]) {
        if self.entries.len() >= MAX_ENTRIES && !self.entries.contains_key(&local_id) {
            return;
        }
        let same = self.entries.get(&local_id).is_some_and(|e| e.crc == crc && e.data == data);
        if !same {
            self.entries.insert(
                local_id,
                Entry {
                    crc,
                    update_flags,
                    data: data.to_vec(),
                },
            );
            self.dirty = true;
        }
    }

    /// Override message payload of an object, replayed on a cache hit.
    pub fn gltf_override(&self, local_id: u32) -> Option<&[u8]> {
        self.overrides.get(&local_id).map(Vec::as_slice)
    }

    /// Latest override message of an object (each one replaces the previous;
    /// `None` when it cleared them: LLViewerRegion::cacheFullUpdateGLTFOverride).
    pub fn set_gltf_override(&mut self, local_id: u32, payload: Option<&[u8]>) {
        let changed = match payload {
            Some(p) if self.overrides.get(&local_id).map(Vec::as_slice) == Some(p) => false,
            Some(p) => {
                self.overrides.insert(local_id, p.to_vec());
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
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
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
        c.save();
        let c2 = RegionObjectCache::load(&dir, region);
        assert_eq!(c2.len(), 2);
        assert_eq!(c2.get(42, 7).map(|e| e.data.clone()), Some(vec![1, 2, 3]));
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
}
