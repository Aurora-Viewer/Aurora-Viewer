//! What is held of a mesh asset: the parts of it downloaded or read so far,
//! each at its place in the asset, and their disk cache file.
//!
//! A mesh is no longer downloaded whole: its header first, then only the
//! blocks that are drawn (a level of detail, the skin), as Firestorm does
//! (`LLMeshRepoThread::fetchMeshHeader` / `fetchMeshLOD` /
//! `fetchMeshSkinInfo`, llmeshrepository.cpp). Each reply is kept as it
//! came, never copied into a larger buffer: a block is read from the part
//! that holds all of it.
//!
//! Cache file: a whole asset is written as it is (as before, so older cache
//! files stay valid); a partial one as `AMSH`, a version byte, the number of
//! parts, then each part as its start, its length and its bytes (little
//! endian). Firestorm writes the blocks at their offsets in a sparse file
//! with flags for those present; one file of parts does the same without
//! holes.

use std::path::Path;
use std::sync::{Arc, Mutex};

const MAGIC: &[u8; 4] = b"AMSH";
const VERSION: u8 = 1;
/// A cache file is not believed past this many parts.
const MOST_PARTS: usize = 64;

/// Parts of a mesh asset, by their start in the asset.
#[derive(Clone, Debug, Default)]
pub struct MeshBytes {
    parts: Vec<(usize, Arc<Vec<u8>>)>,
    /// The whole asset is held (downloaded or read in one piece).
    whole: bool,
}

impl MeshBytes {
    pub fn whole(data: Arc<Vec<u8>>) -> Self {
        MeshBytes {
            parts: vec![(0, data)],
            whole: true,
        }
    }

    pub fn is_whole(&self) -> bool {
        self.whole
    }

    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// Bytes held, overlaps counted twice.
    pub fn held(&self) -> usize {
        self.parts.iter().map(|(_, p)| p.len()).sum()
    }

    /// Keep a part: one that another holds already is dropped, those it
    /// holds go.
    pub fn add(&mut self, start: usize, data: Arc<Vec<u8>>) {
        if data.is_empty() {
            return;
        }
        let end = start + data.len();
        if self.parts.iter().any(|(s, p)| *s <= start && s + p.len() >= end) {
            return;
        }
        self.parts.retain(|(s, p)| !(*s >= start && s + p.len() <= end));
        let at = self.parts.partition_point(|(s, _)| *s < start);
        self.parts.insert(at, (start, data));
    }

    /// The bytes of `(start, len)`, when one part holds all of them.
    pub fn section(&self, (start, len): (usize, usize)) -> Option<&[u8]> {
        let end = start.checked_add(len)?;
        self.parts
            .iter()
            .find(|(s, p)| *s <= start && s + p.len() >= end)
            .map(|(s, p)| &p[start - s..end - s])
    }

    pub fn holds(&self, range: (usize, usize)) -> bool {
        self.section(range).is_some()
    }

    pub fn into_parts(self) -> Vec<(usize, Arc<Vec<u8>>)> {
        self.parts
    }

    /// Add what `other` holds: a whole asset holds everything.
    pub fn merge(&mut self, other: MeshBytes) {
        if self.whole {
            return;
        }
        if other.whole {
            *self = other;
            return;
        }
        for (start, data) in other.parts {
            self.add(start, data);
        }
    }

    /// Write what is held to the cache file at `path`, with what the file
    /// already holds: a file is never replaced by less (a whole asset by a
    /// few blocks, after a read of it that failed; or the blocks of a later
    /// write by those of an earlier one finishing after it, the writes being
    /// jobs). Nothing is written when the file already holds it all.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        // (one write at a time: the read and the write of two jobs on the
        // same file must not cross)
        static WRITING: Mutex<()> = Mutex::new(());
        let _one = WRITING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(mut held) = std::fs::read(path).ok().and_then(MeshBytes::decode) else {
            return crate::cache::write(path, &self.encode());
        };
        let before = (held.whole, held.parts.len(), held.held());
        held.merge(self.clone());
        if (held.whole, held.parts.len(), held.held()) == before {
            return Ok(());
        }
        crate::cache::write(path, &held.encode())
    }

    /// The part that starts the asset (its header).
    pub fn first(&self) -> Option<&[u8]> {
        self.parts.first().filter(|(s, _)| *s == 0).map(|(_, p)| p.as_slice())
    }

    /// The cache file of what is held.
    pub fn encode(&self) -> Vec<u8> {
        if self.whole
            && let Some((0, data)) = self.parts.first()
        {
            return data.as_ref().clone();
        }
        let mut out = Vec::with_capacity(self.held() + 9 + 16 * self.parts.len());
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.extend_from_slice(&(self.parts.len() as u32).to_le_bytes());
        for (start, data) in &self.parts {
            out.extend_from_slice(&(*start as u64).to_le_bytes());
            out.extend_from_slice(&(data.len() as u64).to_le_bytes());
            out.extend_from_slice(data);
        }
        out
    }

    /// What a cache file holds: its parts, or a whole asset (any file
    /// without the magic, as written before parts were kept). None when the
    /// file is damaged.
    pub fn decode(file: Vec<u8>) -> Option<Self> {
        if !file.starts_with(MAGIC) {
            return (!file.is_empty()).then(|| MeshBytes::whole(Arc::new(file)));
        }
        let mut at = MAGIC.len();
        let take = |at: &mut usize, n: usize| -> Option<&[u8]> {
            let bytes = file.get(*at..at.checked_add(n)?)?;
            *at += n;
            Some(bytes)
        };
        if take(&mut at, 1)? != [VERSION] {
            return None;
        }
        let count = u32::from_le_bytes(take(&mut at, 4)?.try_into().ok()?) as usize;
        if count > MOST_PARTS {
            return None;
        }
        let mut bytes = MeshBytes::default();
        for _ in 0..count {
            let start = u64::from_le_bytes(take(&mut at, 8)?.try_into().ok()?);
            let len = u64::from_le_bytes(take(&mut at, 8)?.try_into().ok()?);
            let data = take(&mut at, usize::try_from(len).ok()?)?;
            bytes.add(usize::try_from(start).ok()?, Arc::new(data.to_vec()));
        }
        (at == file.len() && !bytes.is_empty()).then_some(bytes)
    }
}

/// Byte ranges to download for `blocks` (start, length) that are not held,
/// joined when the gap between two is at most `join_gap` bytes (one request
/// instead of two, for a few bytes more), sorted by start.
pub fn ranges_to_fetch(held: &MeshBytes, blocks: &[(usize, usize)], join_gap: usize) -> Vec<(usize, usize)> {
    let mut missing: Vec<(usize, usize)> = blocks.iter().copied().filter(|b| b.1 > 0 && !held.holds(*b)).collect();
    missing.sort_unstable();
    missing.dedup();
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (start, len) in missing {
        if let Some(last) = out.last_mut() {
            let last_end = last.0 + last.1;
            if start <= last_end + join_gap {
                last.1 = (start + len).max(last_end) - last.0;
                continue;
            }
        }
        out.push((start, len));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(start: u8, len: usize) -> Arc<Vec<u8>> {
        Arc::new((0..len).map(|i| start.wrapping_add(i as u8)).collect())
    }

    #[test]
    fn a_block_is_read_from_the_part_that_holds_it() {
        let mut b = MeshBytes::default();
        b.add(0, part(0, 4096));
        b.add(10_000, part(16, 500));
        assert_eq!(b.section((100, 4)), Some(&[100, 101, 102, 103][..]));
        assert_eq!(b.section((10_010, 2)), Some(&[26, 27][..]));
        // across the end of a part, or between two: not held
        assert!(!b.holds((4000, 200)));
        assert!(!b.holds((9_999, 2)));
        assert_eq!(b.first().map(<[u8]>::len), Some(4096));
        assert!(!b.is_whole());
    }

    #[test]
    fn parts_held_by_others_are_dropped() {
        let mut b = MeshBytes::default();
        b.add(100, part(0, 10));
        b.add(105, part(0, 2));
        assert_eq!(b.parts.len(), 1);
        b.add(0, part(0, 1000));
        assert_eq!(b.parts.len(), 1);
        assert_eq!(b.held(), 1000);
    }

    #[test]
    fn the_cache_file_keeps_the_parts_or_the_whole_asset() {
        let mut b = MeshBytes::default();
        b.add(0, part(0, 300));
        b.add(5000, part(7, 40));
        let back = MeshBytes::decode(b.encode()).expect("parts");
        assert_eq!(back.section((5010, 3)), b.section((5010, 3)));
        assert_eq!(back.first(), b.first());
        // a whole asset is written as it is, and read back as one
        let whole = MeshBytes::whole(part(0x7b, 64));
        assert_eq!(whole.encode(), *part(0x7b, 64));
        assert!(MeshBytes::decode(whole.encode()).expect("whole").is_whole());
        // damaged: cut, or too many parts
        let mut cut = b.encode();
        cut.truncate(cut.len() - 1);
        assert!(MeshBytes::decode(cut).is_none());
        let mut many = b"AMSH\x01".to_vec();
        many.extend_from_slice(&1000u32.to_le_bytes());
        assert!(MeshBytes::decode(many).is_none());
        assert!(MeshBytes::decode(Vec::new()).is_none());
    }

    /// A cache file is never replaced by less: a whole asset stays whole
    /// when a few blocks of it are written after a read of it that failed,
    /// and blocks written first are kept when an earlier write, with fewer
    /// of them, lands after.
    #[test]
    fn a_cache_write_never_replaces_more_with_less() {
        let dir = std::env::temp_dir().join(format!("aurora-mesh-save-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("asset.mesh");
        let read = || MeshBytes::decode(std::fs::read(&path).expect("file")).expect("cache file");

        // a whole asset on disk, then a few of its blocks
        let whole = part(0, 6000);
        MeshBytes::whole(whole.clone()).save(&path).expect("whole");
        let mut blocks = MeshBytes::default();
        blocks.add(0, Arc::new(whole[..300].to_vec()));
        blocks.add(5000, Arc::new(whole[5000..5040].to_vec()));
        blocks.save(&path).expect("blocks");
        let back = read();
        assert!(back.is_whole(), "the whole asset was replaced by blocks");
        assert_eq!(back.held(), 6000);

        // parts: the later write (more) first, the earlier one (less) after
        std::fs::remove_file(&path).expect("remove");
        let mut later = MeshBytes::default();
        later.add(0, part(0, 300));
        later.add(5000, part(7, 40));
        let mut earlier = MeshBytes::default();
        earlier.add(0, part(0, 300));
        later.save(&path).expect("later");
        earlier.save(&path).expect("earlier");
        assert!(read().holds((5000, 40)), "the earlier write dropped a block");
        // and a block not held yet is added to the file
        let mut other = MeshBytes::default();
        other.add(9000, part(3, 10));
        other.save(&path).expect("other");
        let back = read();
        assert!(back.holds((9000, 10)) && back.holds((5000, 40)) && back.holds((0, 300)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_blocks_are_joined_when_close() {
        let mut held = MeshBytes::default();
        held.add(0, part(0, 4096));
        // skin inside the header's reply; two blocks 1 KB apart; one far
        let blocks = [(3000, 500), (10_000, 2000), (13_000, 100), (50_000, 10), (10_000, 2000)];
        assert_eq!(ranges_to_fetch(&held, &blocks, 4096), [(10_000, 3100), (50_000, 10)]);
        assert_eq!(ranges_to_fetch(&held, &blocks, 0), [(10_000, 2000), (13_000, 100), (50_000, 10)]);
    }
}
