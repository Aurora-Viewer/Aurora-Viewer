//! Disk cache of transcoded textures (`<cache>/tex/<id>.bc7`): the BC7
//! mip chain a decode job produced, with what the streamer learns from a
//! decode (alpha class, pick mask, size of the JPEG 2000 image). Back in a
//! region, the texture is read from there and goes to the GPU as it is: no
//! JPEG 2000 decode, no mips, no BC7 encoding (190 s of processor for an
//! arrival at a busy store on the grid, TASKS.md).
//!
//! One file per texture, holding the finest level decoded so far, largest
//! level first: a coarser level is read from the middle of the file (its
//! mips are those of the finer chain), a finer one needs the JPEG 2000 data
//! again and rewrites the file. A texture that BC7 would harm
//! (`aurora_render::bc7`) leaves a file without levels, so that the
//! encoding is not tried again at each decode.
//!
//! The file is ours but it is still data from the disk: every size is
//! checked against the file and against what a texture can be.

use super::jobs::AlphaKind;
use aurora_assets::J2kInfo;
use aurora_render::TexelFormat;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const MAGIC: &[u8; 4] = b"AUTX";
/// Bumped when the layout, the encoder profile or its quality check change:
/// incompatible files are rewritten by the next decode. 2: the
/// colour of the mips is the plain average of the texels (`build_mips`). 3:
/// the ends of alpha stay exact (`aurora_render::bc7::encode_chain`).
/// 4: retain the legacy pick mask and an exact alpha > 0 plane together.
const VERSION: u8 = 4;
const KIND_KEEP_RGBA: u8 = 0;
const KIND_BC7: u8 = 1;
/// Fixed part of the header, then 8 bytes per level.
const HEADER: usize = 32;
const MAX_LEVELS: usize = 16;
const MAX_DIM: u32 = 16384;
/// Legacy 2 × 2 mask plus one bit per texel for world Blend left clicks.
const MAX_MASK_BYTES: usize = (MAX_DIM as usize / 2) * (MAX_DIM as usize / 2) / 8 + MAX_DIM as usize * MAX_DIM as usize / 8;

pub(super) fn pick_mask_bytes(width: u32, height: u32) -> Option<usize> {
    if width == 0 || height == 0 || width > MAX_DIM || height > MAX_DIM {
        return None;
    }
    Some((width.div_ceil(2) as usize * height.div_ceil(2) as usize).div_ceil(8) + (width as usize * height as usize).div_ceil(8))
}

/// Files waiting to be written may hold this much; past it, new ones are
/// dropped (their texture is decoded again next time).
const WRITE_QUEUE_BYTES: u64 = 256 << 20;

/// Writes the files on a thread of its own. Creating a file takes
/// milliseconds (15 ms each when thirty decode jobs did it at once, more
/// than the decodes themselves): a decode job only hands its file over.
#[derive(Clone)]
pub struct Writer {
    tx: crossbeam_channel::Sender<(PathBuf, Vec<u8>)>,
    queued: Arc<AtomicU64>,
}

impl Writer {
    /// None when the thread cannot start: nothing is cached.
    pub fn start() -> Option<Self> {
        let (tx, rx) = crossbeam_channel::unbounded::<(PathBuf, Vec<u8>)>();
        let queued = Arc::new(AtomicU64::new(0));
        let left = queued.clone();
        let spawned = std::thread::Builder::new().name("aurora-texcache".into()).spawn(move || {
            for (path, file) in rx {
                // (written apart then renamed: a reader never sees a part of it)
                let _ = crate::cache::write(&path, &file);
                left.fetch_sub(file.len() as u64, Ordering::Relaxed);
            }
        });
        match spawned {
            Ok(_) => Some(Writer { tx, queued }),
            Err(e) => {
                log::warn!("transcoded texture cache: writer thread: {e}");
                None
            }
        }
    }

    pub fn write(&self, path: PathBuf, file: Vec<u8>) {
        let len = file.len() as u64;
        if self.queued.load(Ordering::Relaxed) + len > WRITE_QUEUE_BYTES {
            return;
        }
        self.queued.fetch_add(len, Ordering::Relaxed);
        if self.tx.send((path, file)).is_err() {
            self.queued.fetch_sub(len, Ordering::Relaxed);
        }
    }
}

/// A pick mask: full texel dimensions, then legacy and exact bit planes.
pub type MaskParts = (u32, u32, Vec<u8>);

/// What a decode leaves in the cache besides the levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meta {
    /// The JPEG 2000 image the texture comes from.
    pub info: J2kInfo,
    /// Discard level of the first level stored.
    pub discard: u8,
    pub alpha: AlphaKind,
    pub alpha_channel: bool,
}

/// A transcoded texture read back, from the level asked.
#[derive(Debug, PartialEq, Eq)]
pub struct Cached {
    pub info: J2kInfo,
    /// Finest discard level the file holds.
    pub finest: u8,
    /// Discard level of `levels[0]`.
    pub discard: u8,
    pub alpha: AlphaKind,
    pub alpha_channel: bool,
    pub pick_mask: Option<MaskParts>,
    /// BC7 blocks: (width, height, data), largest first.
    pub levels: Vec<(u32, u32, Vec<u8>)>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Transcoded {
    Bc7(Cached),
    /// BC7 would harm this texture: decode it and keep it RGBA8.
    KeepRgba,
}

/// Alpha class as stored. A mask's mips depend on whether its alpha
/// coverage is kept (`super::textures::alpha_coverage_on`): 3 when it is, 1
/// when not, so that a file made the other way reads as missing and its
/// texture is decoded again; the other textures' files stay valid.
fn alpha_code(a: AlphaKind) -> u8 {
    match a {
        AlphaKind::Opaque => 0,
        AlphaKind::Mask if super::textures::alpha_coverage_on() => 3,
        AlphaKind::Mask => 1,
        AlphaKind::Blend => 2,
    }
}

fn header(kind: u8, meta: &Meta, levels: usize, mask: Option<(u32, u32, usize)>) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + levels * 8);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&[
        VERSION,
        kind,
        meta.discard,
        alpha_code(meta.alpha),
        u8::from(meta.alpha_channel),
        meta.info.components,
        meta.info.levels,
        levels as u8,
    ]);
    out.extend_from_slice(&meta.info.width.to_le_bytes());
    out.extend_from_slice(&meta.info.height.to_le_bytes());
    let (mw, mh, mlen) = mask.unwrap_or((0, 0, 0));
    out.extend_from_slice(&mw.to_le_bytes());
    out.extend_from_slice(&mh.to_le_bytes());
    out.extend_from_slice(&(mlen as u32).to_le_bytes());
    debug_assert_eq!(out.len(), HEADER);
    out
}

/// The file of a BC7 chain (level 0 first, as `bc7::encode_chain` gives it).
/// None for a chain that cannot be stored (too many levels, a short level).
pub fn encode(meta: &Meta, pick_mask: Option<(u32, u32, &[u8])>, chain: &[(u32, u32, Vec<u8>)]) -> Option<Vec<u8>> {
    let complete = chain.iter().all(|(w, h, d)| d.len() as u64 == TexelFormat::Bc7.level_bytes(*w, *h));
    if chain.is_empty() || chain.len() > MAX_LEVELS || !complete {
        return None;
    }
    let mask = pick_mask.filter(|m| m.2.len() <= MAX_MASK_BYTES);
    let mut out = header(KIND_BC7, meta, chain.len(), mask.map(|m| (m.0, m.1, m.2.len())));
    for (w, h, _) in chain {
        out.extend_from_slice(&w.to_le_bytes());
        out.extend_from_slice(&h.to_le_bytes());
    }
    if let Some((_, _, bits)) = mask {
        out.extend_from_slice(bits);
    }
    for (_, _, data) in chain {
        out.extend_from_slice(data);
    }
    Some(out)
}

/// The file of a texture that stays RGBA8.
pub fn encode_keep_rgba(meta: &Meta) -> Vec<u8> {
    header(KIND_KEEP_RGBA, meta, 0, None)
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Read a transcoded texture at discard level `want`, or at the finest
/// level it holds when that is coarser. None for a missing, older or
/// damaged file.
pub fn read(path: &Path, want: u8) -> Option<Transcoded> {
    read_for(path, |_| want)
}

/// Coarse preview reads stay bounded even before the streamer knows the
/// texture's dimensions. Select from the cache header before loading blocks.
pub fn read_preview(path: &Path) -> Option<Transcoded> {
    read_for(path, |head| {
        preview_discard(u32_at(head, 12).unwrap_or(0), u32_at(head, 16).unwrap_or(0))
    })
}
pub fn preview_discard(mut width: u32, mut height: u32) -> u8 {
    let mut discard = 0;
    while width.max(height) > 256 && discard < 15 {
        (width, height) = (width.div_ceil(2), height.div_ceil(2));
        discard += 1;
    }
    discard
}
fn read_for(path: &Path, want: impl FnOnce(&[u8]) -> u8) -> Option<Transcoded> {
    let mut file = std::fs::File::open(path).ok()?;
    let file_len = file.metadata().ok()?.len();
    let mut head = vec![0u8; (HEADER + MAX_LEVELS * 8).min(file_len as usize)];
    file.read_exact(&mut head).ok()?;
    let parsed = parse(&head, file_len, want(&head))?;
    match parsed {
        Parsed::KeepRgba => Some(Transcoded::KeepRgba),
        Parsed::Bc7 { mut cached, mask, data } => {
            if let Some((w, h, at, len)) = mask {
                let mut bits = vec![0u8; len];
                file.seek(SeekFrom::Start(at)).ok()?;
                file.read_exact(&mut bits).ok()?;
                cached.pick_mask = Some((w, h, bits));
            }
            file.seek(SeekFrom::Start(data)).ok()?;
            for (w, h, blocks) in &mut cached.levels {
                *blocks = vec![0u8; TexelFormat::Bc7.level_bytes(*w, *h) as usize];
                file.read_exact(blocks).ok()?;
                crate::disk_io::read(crate::disk_io::Origin::Textures, blocks.len());
            }
            // (least recently used files go first when the cache is full)
            crate::cache::touch(path);
            Some(Transcoded::Bc7(cached))
        }
    }
}

enum Parsed {
    KeepRgba,
    Bc7 {
        /// Levels sized, not read yet.
        cached: Cached,
        /// Pick mask: (width, height, offset, bytes).
        mask: Option<(u32, u32, u64, usize)>,
        /// Offset of the first level to read.
        data: u64,
    },
}

/// The header checked against the length of the file, and where to read
/// the levels from `want` on.
fn parse(head: &[u8], file_len: u64, want: u8) -> Option<Parsed> {
    if head.get(..4)? != MAGIC || !matches!(*head.get(4)?, 3 | VERSION) {
        return None;
    }
    let kind = head[5];
    let stored = head[6];
    let alpha = match head[7] {
        0 => AlphaKind::Opaque,
        2 => AlphaKind::Blend,
        code if code == alpha_code(AlphaKind::Mask) => AlphaKind::Mask,
        _ => return None,
    };
    let info = J2kInfo {
        width: u32_at(head, 12)?,
        height: u32_at(head, 16)?,
        components: head[9],
        levels: head[10],
    };
    if info.width == 0 || info.height == 0 || info.width > MAX_DIM || info.height > MAX_DIM || stored > 5 {
        return None;
    }
    if kind == KIND_KEEP_RGBA {
        return Some(Parsed::KeepRgba);
    }
    let count = head[11] as usize;
    if kind != KIND_BC7 || count == 0 || count > MAX_LEVELS {
        return None;
    }
    let (mw, mh, mlen) = (u32_at(head, 20)?, u32_at(head, 24)?, u32_at(head, 28)? as usize);
    // Opaque version-3 chains remain usable. Alpha masks need a fresh decode
    // once to supply the exact bit plane rather than guessing transparency.
    if mlen > MAX_MASK_BYTES || (mlen > 0 && (head[4] != VERSION || pick_mask_bytes(mw, mh) != Some(mlen))) {
        return None;
    }
    let mut sizes = Vec::with_capacity(count);
    for i in 0..count {
        let (w, h) = (u32_at(head, HEADER + i * 8)?, u32_at(head, HEADER + i * 8 + 4)?);
        if w == 0 || h == 0 || w > MAX_DIM || h > MAX_DIM {
            return None;
        }
        sizes.push((w, h));
    }
    // a chain: whole blocks at level 0, each level half the one before
    let (w0, h0) = sizes[0];
    let halves = sizes
        .iter()
        .enumerate()
        .all(|(i, &(w, h))| w == (w0 >> i).max(1) && h == (h0 >> i).max(1));
    if !TexelFormat::Bc7.fits(w0, h0) || !halves {
        return None;
    }
    let mask_at = (HEADER + count * 8) as u64;
    let first = mask_at + mlen as u64;
    let bytes = |&(w, h): &(u32, u32)| TexelFormat::Bc7.level_bytes(w, h);
    if first + sizes.iter().map(bytes).sum::<u64>() != file_len {
        return None;
    }
    let skip = (want.saturating_sub(stored) as usize).min(count - 1);
    Some(Parsed::Bc7 {
        cached: Cached {
            info,
            finest: stored,
            discard: stored + skip as u8,
            alpha,
            alpha_channel: head[8] != 0,
            pick_mask: None,
            levels: sizes[skip..].iter().map(|&(w, h)| (w, h, Vec::new())).collect(),
        },
        mask: (mlen > 0).then_some((mw, mh, mask_at, mlen)),
        data: first + sizes[..skip].iter().map(bytes).sum::<u64>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(discard: u8) -> Meta {
        Meta {
            info: J2kInfo {
                width: 64,
                height: 32,
                components: 4,
                levels: 5,
            },
            discard,
            alpha: AlphaKind::Mask,
            alpha_channel: true,
        }
    }

    /// A chain of 16 × 8 down to 1 × 1, each block filled with its level.
    fn chain() -> Vec<(u32, u32, Vec<u8>)> {
        (0..5u32)
            .map(|l| {
                let (w, h) = ((16 >> l).max(1), (8 >> l).max(1));
                (w, h, vec![l as u8 + 1; TexelFormat::Bc7.level_bytes(w, h) as usize])
            })
            .collect()
    }

    fn file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("aurora-texture-cache-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("test folder");
        let path = dir.join(name);
        std::fs::write(&path, bytes).expect("test file");
        path
    }

    #[test]
    fn a_chain_comes_back_whole_at_its_level() {
        let mask = vec![0b1010_1010u8; pick_mask_bytes(8, 4).unwrap()];
        let bytes = encode(&meta(2), Some((8, 4, &mask)), &chain()).expect("file");
        let path = file("whole.bc7", &bytes);
        let Some(Transcoded::Bc7(c)) = read(&path, 2) else {
            panic!("a BC7 chain");
        };
        assert_eq!((c.info, c.finest, c.discard), (meta(2).info, 2, 2));
        assert_eq!((c.alpha, c.alpha_channel), (AlphaKind::Mask, true));
        assert_eq!(c.pick_mask, Some((8, 4, mask)));
        assert_eq!(c.levels, chain());
        // a finer level than the file holds: its finest
        assert_eq!(read(&path, 0), read(&path, 2));
    }

    #[test]
    fn a_coarser_level_is_the_end_of_the_chain() {
        let path = file("coarser.bc7", &encode(&meta(1), None, &chain()).expect("file"));
        let Some(Transcoded::Bc7(c)) = read(&path, 3) else {
            panic!("a BC7 chain");
        };
        assert_eq!((c.finest, c.discard, c.pick_mask.is_none()), (1, 3, true));
        assert_eq!(c.levels, chain()[2..]);
        // past the last level: the 1 × 1 one
        let Some(Transcoded::Bc7(c)) = read(&path, 5) else {
            panic!("a BC7 chain");
        };
        assert_eq!((c.discard, c.levels.len()), (5, 1));
    }

    #[test]
    fn a_harmed_texture_leaves_a_file_without_levels() {
        let path = file("keep.bc7", &encode_keep_rgba(&meta(0)));
        assert_eq!(read(&path, 0), Some(Transcoded::KeepRgba));
    }

    #[test]
    fn old_opaque_chains_survive_but_old_alpha_masks_are_decoded_again() {
        let opaque_meta = Meta {
            alpha: AlphaKind::Opaque,
            ..meta(0)
        };
        let mut opaque = encode(&opaque_meta, None, &chain()).unwrap();
        opaque[4] = 3;
        assert!(matches!(read(&file("old-opaque.bc7", &opaque), 0), Some(Transcoded::Bc7(_))));
        let mut masked = encode(&meta(0), Some((8, 4, &[0; 5])), &chain()).unwrap();
        masked[4] = 3;
        assert_eq!(read(&file("old-mask.bc7", &masked), 0), None);
        assert_eq!(pick_mask_bytes(u32::MAX, u32::MAX), None);
    }

    #[test]
    fn damaged_files_are_ignored() {
        let good = encode(&meta(0), Some((8, 4, &[0u8; 5])), &chain()).expect("file");
        let check = |name: &str, change: &dyn Fn(&mut Vec<u8>)| {
            let mut bytes = good.clone();
            change(&mut bytes);
            assert_eq!(read(&file(name, &bytes), 0), None, "{name}");
        };
        check("magic", &|b| b[0] = b'X');
        check("version", &|b| b[4] = VERSION + 1);
        check("kind", &|b| b[5] = 9);
        check("alpha", &|b| b[7] = 4);
        // a mask whose mips were made without keeping its alpha coverage
        // (files of before, or AURORA_ALPHA_COVERAGE=0): decoded again
        check("mask-mips", &|b| b[7] = 1);
        check("levels", &|b| b[11] = 17);
        check("no-level", &|b| b[11] = 0);
        check("short", &|b| b.truncate(b.len() - 1));
        check("long", &|b| b.push(0));
        check("header-only", &|b| b.truncate(20));
        check("mask-size", &|b| b[28] = 6);
        check("level-size", &|b| b[HEADER + 8] = 9);
        check("odd-size", &|b| b[HEADER] = 18);
        check("huge", &|b| b[HEADER + 2] = 0xFF);
        assert_eq!(read(Path::new("no such file.bc7"), 0), None);
        // what cannot be stored is refused when writing
        assert!(encode(&meta(0), None, &[]).is_none());
        assert!(encode(&meta(0), None, &[(16, 8, vec![0; 3])]).is_none());
    }
}
