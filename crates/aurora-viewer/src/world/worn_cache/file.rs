//! The file of one avatar's worn objects: what [`super::capture`] keeps of
//! them, read back by [`decode`]. One sealed store file per avatar
//! (`objects/<name>.worn`, the name derived from the avatar id like every
//! store file), so that the cache's size limit and Clear Cache take them
//! with the region object caches.
//!
//! Layout (little-endian): `AWC1`, the time it was written (Unix seconds,
//! u64), the number of objects (u16); each object: its root's id at the
//! time (16 bytes), its inventory item (16, nil when unknown), its
//! attachment point (u8), the number of prims (u16); each prim, the root
//! first: its update flags (u32), its compressed update
//! (`aurora_net::objects::encode_compressed`, u32 length then the bytes),
//! its GLTF overrides (LLSD binary of an array of `[face, override]`, u32
//! length, empty when it has none).

use uuid::Uuid;

const MAGIC: &[u8; 4] = b"AWC1";
/// Bounds of what a file may hold (an outfit is a few hundred prims).
pub const MAX_OBJECTS: usize = 80;
pub const MAX_PRIMS: usize = 2048;
const MAX_PRIM_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WornSet {
    /// Unix seconds.
    pub saved_at: u64,
    pub objects: Vec<WornObject>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WornObject {
    /// The root's id when it was kept: an object's id changes each time it
    /// is attached, so it only matches while its wearer stays logged in.
    pub root_id: Uuid,
    /// Its inventory item (AttachItemID), nil when it was not known.
    pub item_id: Uuid,
    pub point: u8,
    /// The root first, then its children.
    pub prims: Vec<WornPrim>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WornPrim {
    pub update_flags: u32,
    pub data: Vec<u8>,
    pub overrides: Vec<u8>,
}

impl WornSet {
    pub fn prim_count(&self) -> usize {
        self.objects.iter().map(|o| o.prims.len()).sum()
    }
}

pub fn encode(set: &WornSet) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + set.prim_count() * 320);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&set.saved_at.to_le_bytes());
    let objects = &set.objects[..set.objects.len().min(MAX_OBJECTS)];
    out.extend_from_slice(&(objects.len() as u16).to_le_bytes());
    for o in objects {
        out.extend_from_slice(o.root_id.as_bytes());
        out.extend_from_slice(o.item_id.as_bytes());
        out.push(o.point);
        let prims = &o.prims[..o.prims.len().min(MAX_PRIMS)];
        out.extend_from_slice(&(prims.len() as u16).to_le_bytes());
        for p in prims {
            out.extend_from_slice(&p.update_flags.to_le_bytes());
            for blob in [&p.data, &p.overrides] {
                out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
                out.extend_from_slice(blob);
            }
        }
    }
    out
}

struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.d.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(s)
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn uuid(&mut self) -> Option<Uuid> {
        Uuid::from_slice(self.take(16)?).ok()
    }
    fn blob(&mut self) -> Option<Vec<u8>> {
        let n = self.u32()? as usize;
        if n > MAX_PRIM_BYTES {
            return None;
        }
        Some(self.take(n)?.to_vec())
    }
}

/// A file of [`encode`]; anything else, truncated or over the bounds, is
/// none (a cache miss).
pub fn decode(d: &[u8]) -> Option<WornSet> {
    let mut r = Reader { d, p: 0 };
    if r.take(4)? != MAGIC {
        return None;
    }
    let saved_at = r.u64()?;
    let count = r.u16()? as usize;
    if count > MAX_OBJECTS {
        return None;
    }
    let mut objects = Vec::with_capacity(count);
    let mut total = 0usize;
    for _ in 0..count {
        let root_id = r.uuid()?;
        let item_id = r.uuid()?;
        let point = r.take(1)?[0];
        let n = r.u16()? as usize;
        total += n;
        if n == 0 || total > MAX_PRIMS {
            return None;
        }
        let mut prims = Vec::with_capacity(n);
        for _ in 0..n {
            prims.push(WornPrim {
                update_flags: r.u32()?,
                data: r.blob()?,
                overrides: r.blob()?,
            });
        }
        objects.push(WornObject {
            root_id,
            item_id,
            point,
            prims,
        });
    }
    (r.p == d.len()).then_some(WornSet { saved_at, objects })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> WornSet {
        WornSet {
            saved_at: 1_763_500_000,
            objects: vec![
                WornObject {
                    root_id: Uuid::from_u128(1),
                    item_id: Uuid::from_u128(2),
                    point: 2,
                    prims: vec![
                        WornPrim {
                            update_flags: 0x10,
                            data: vec![1, 2, 3],
                            overrides: Vec::new(),
                        },
                        WornPrim {
                            update_flags: 0,
                            data: vec![4; 300],
                            overrides: vec![9, 9],
                        },
                    ],
                },
                WornObject {
                    root_id: Uuid::from_u128(3),
                    item_id: Uuid::nil(),
                    point: 40,
                    prims: vec![WornPrim {
                        update_flags: 0,
                        data: vec![5],
                        overrides: Vec::new(),
                    }],
                },
            ],
        }
    }

    #[test]
    fn a_worn_set_reads_back() {
        let set = sample();
        let bytes = encode(&set);
        assert_eq!(decode(&bytes), Some(set));
        assert_eq!(decode(&encode(&WornSet::default())), Some(WornSet::default()));
    }

    #[test]
    fn a_damaged_file_is_a_miss() {
        let bytes = encode(&sample());
        for n in 0..bytes.len() {
            assert_eq!(decode(&bytes[..n]), None, "truncated at {n}");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert_eq!(decode(&longer), None);
        let mut other = bytes;
        other[0] = b'X';
        assert_eq!(decode(&other), None);
    }
}
