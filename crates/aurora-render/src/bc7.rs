//! BC7 compression of the streamed textures: the RGBA8 mip chain a decode
//! job built becomes blocks of 4×4 texels — a quarter of the video memory,
//! and of the copy to the GPU — when the texture comes out of it unharmed.
//!
//! Encoder: Intel's ISPC texture compressor (`intel_tex_2`), "ultra fast"
//! profile: about 70 megapixels a second on one thread with the check, a
//! third of what the JPEG 2000 decode before it costs. Measured over a viewer's cache
//! (TASKS.md), that profile is transparent for most textures (median PSNR
//! 53 dB) but visibly harms about one in ten: normal maps with a fine
//! grain, thin lattices with alpha. The slower profiles do better on those
//! but cost nine times more. So each texture is checked instead
//! ([`harmed`]): blocks of every level are decoded back and compared with
//! the source, and a texture with too many blocks off stays RGBA8, as before.
//! About 85 % of the textures pass, for 2.9 times less video memory overall
//! (`cargo run --release -p aurora-render --example bc7_check -- <cache/tex>`).
//! With alpha, the texels at an end of alpha (0 or 255) are kept exact on
//! top of that, in every block ([`mend_alpha_ends`]): a mask cutoff at an
//! end turns the smallest error there into holes.

use intel_tex_2::{RgbaSurface, bc7};

/// A texel channel off by more than this (of 255) puts its block off. With
/// alpha, color errors count as much as the texel shows (weighted by the
/// source alpha) and the alpha error counts whole.
const TOLERANCE: u32 = 16;
/// Share of the checked blocks that may be off, per thousand.
const MAX_OFF_PER_MILLE: usize = 10;
/// One block in this many is checked (all of them in a small level).
const CHECK_ONE_IN: usize = 4;
const CHECK_ALL_UNDER: usize = 1024;
/// Levels under this many whole blocks (32 × 32 texels) are not checked.
const MIN_BLOCKS: usize = 64;

/// A mip chain: (width, height, data) per level, level 0 first.
pub type Chain = Vec<(u32, u32, Vec<u8>)>;

/// The BC7 blocks of an RGBA8 mip chain (as [`crate::build_mips`] gives it),
/// with the same level sizes in texels. `alpha`: the texture has
/// see-through texels. None when level 0 is not made of whole blocks, when
/// a level is short, or when the texture would be harmed.
pub fn encode_chain(mips: &[(u32, u32, Vec<u8>)], alpha: bool) -> Option<Chain> {
    let (w, h, _) = mips.first()?;
    if *w == 0 || *h == 0 || !w.is_multiple_of(4) || !h.is_multiple_of(4) {
        return None;
    }
    if mips.iter().any(|(w, h, d)| d.len() < *w as usize * *h as usize * 4) {
        return None;
    }
    // One color line per block (mode 6), alpha on that line too: enough for
    // most textures. With alpha, the blocks whose alpha does not follow the
    // color need the modes that code it apart (4 and 5), five times slower:
    // tried only when the first pass harms the texture, which gives the same
    // textures compressed as using them from the start for a quarter of the
    // time (bc7_check: 89 % of the alpha textures either way).
    let single_line = bc7::EncodeSettings {
        channels: if alpha { 4 } else { 3 },
        ..bc7::opaque_ultra_fast_settings()
    };
    try_chain(&single_line, mips, alpha).or_else(|| alpha.then(|| try_chain(&bc7::alpha_ultra_fast_settings(), mips, alpha)).flatten())
}

/// The chain encoded with `settings`, unless a level comes out harmed. Every
/// level is checked, not only the first: what is on screen is most often a
/// reduced level, and the reduced levels are the harder ones (more detail
/// per block). Level 0 first, which settles most refusals at once.
fn try_chain(settings: &bc7::EncodeSettings, mips: &[(u32, u32, Vec<u8>)], alpha: bool) -> Option<Chain> {
    let mut out = Vec::with_capacity(mips.len());
    for (w, h, rgba) in mips {
        let mut blocks = encode_level(settings, *w, *h, rgba);
        if alpha && !mend_alpha_ends(rgba, &mut blocks, *w as usize, *h as usize) {
            return None;
        }
        if harmed(rgba, &blocks, *w, *h, alpha) {
            return None;
        }
        out.push((*w, *h, blocks));
    }
    Some(out)
}

/// An end of alpha: a texel there must decode to it exactly (see
/// [`mend_alpha_ends`]).
fn alpha_end(a: u8) -> bool {
    a == 0 || a == 255
}

/// Puts back, block by block, the texels at an end of alpha (0 or 255) that
/// the encoding moved; false when a block cannot keep them (the texture
/// then stays RGBA8). A mask cutoff can sit at either end (255: only the
/// opaque texels show; 1: all but the transparent ones), where a texel off
/// by one is a hole or a speck that the tolerance of [`harmed`] lets
/// through. Mode 6 shares the lowest bit of its endpoints between color and
/// alpha: with that bit at 0, its opaque texels decode at 254 (a third of
/// the opaque texels of the compressed alpha textures, holes all over a hat
/// at a cutoff of 255). Its endpoints' bits are set again first (the color
/// moves by 1 of 255 at most); a block they do not mend is encoded again
/// with alpha apart (modes 4 and 5), whose alpha is then set from the
/// texels when the encoder still misses an end ([`snap_alpha`]: in mixed
/// blocks it puts them a few steps in). Every block is looked at, not a
/// sample: one block off is enough to show.
fn mend_alpha_ends(rgba: &[u8], blocks: &mut [u8], w: usize, h: usize) -> bool {
    let columns = w.div_ceil(4);
    let mut decoded = [0u8; 64];
    let mut again: Vec<(usize, [u8; 64])> = Vec::new();
    for by in 0..h.div_ceil(4) {
        for bx in 0..columns {
            let src = source_block(rgba, w, h, bx, by);
            if !src.as_chunks::<4>().0.iter().any(|t| alpha_end(t[3])) {
                continue;
            }
            let at = (by * columns + bx) * 16;
            let Some(block) = blocks.get_mut(at..at + 16) else {
                return false;
            };
            bcdec_rs::bc7(block, &mut decoded, 16);
            if !ends_kept(&src, &decoded) && !mend_mode6(block, &src) {
                again.push((at, src));
            }
        }
    }
    if again.is_empty() {
        return true;
    }
    // the blocks left, stacked in one surface for a single encoder call
    let data: Vec<u8> = again.iter().flat_map(|(_, src)| *src).collect();
    let surface = RgbaSurface {
        data: &data,
        width: 4,
        height: 4 * again.len() as u32,
        stride: 16,
    };
    let apart = bc7::EncodeSettings {
        mode_selection: [false, false, true, false],
        ..bc7::alpha_ultra_fast_settings()
    };
    let mut encoded = bc7::compress_blocks(&apart, &surface);
    for ((at, src), block) in again.iter().zip(encoded.as_chunks_mut::<16>().0) {
        bcdec_rs::bc7(block, &mut decoded, 16);
        if !ends_kept(src, &decoded) {
            if !snap_alpha(block, src) {
                return false;
            }
            bcdec_rs::bc7(block, &mut decoded, 16);
            if !ends_kept(src, &decoded) {
                return false;
            }
        }
        blocks[*at..*at + 16].copy_from_slice(block);
    }
    true
}

/// BC7 interpolation weights, of 64, for 2- and 3-bit indices.
const WEIGHTS2: [u32; 4] = [0, 21, 43, 64];
const WEIGHTS3: [u32; 8] = [0, 9, 18, 27, 37, 46, 55, 64];

/// Sets again the alpha of a mode 4 or 5 block (alpha apart from the color,
/// with endpoints and indices of its own) from the texels' alpha: the
/// lowest and the highest as the endpoints, each texel at the nearest step
/// between them, so that the ends of alpha come out exact. The color is
/// left as the encoder made it. False for another mode, or a block whose
/// alpha is rotated into a color channel.
///
/// Mode 4: 5 mode bits, rotation, index selection, 6 × 5 color bits, 2 × 6
/// alpha bits at 38, 2-bit indices at 50, 3-bit ones at 81 (alpha takes the
/// 3-bit ones unless the selection bit is set). Mode 5: 6 mode bits,
/// rotation, 6 × 7 color bits, 2 × 8 alpha bits at 50, 2-bit color indices
/// at 66, 2-bit alpha indices at 97. The first index of a set has one bit
/// less (its highest is 0).
fn snap_alpha(block: &mut [u8; 16], src: &[u8; 64]) -> bool {
    let mut b = u128::from_le_bytes(*block);
    let get = |b: u128, at: u32, n: u32| ((b >> at) & ((1 << n) - 1)) as u32;
    // (alpha endpoint bits, first index bit, index bits)
    let (end_bits, ends_at, index_at, index_bits) = if b & 0x3f == 0x20 && get(b, 6, 2) == 0 {
        (8, 50, 97, 2)
    } else if b & 0x1f == 0x10 && get(b, 5, 2) == 0 {
        if get(b, 7, 1) == 0 { (6, 38, 81, 3) } else { (6, 38, 50, 2) }
    } else {
        return false;
    };
    let expand = |e: u32| if end_bits == 8 { e } else { (e << 2) | (e >> 4) };
    let quantize = |v: u32| (0..1 << end_bits).min_by_key(|&e| expand(e).abs_diff(v)).unwrap_or(0);
    let alphas: [u32; 16] = std::array::from_fn(|i| u32::from(src[i * 4 + 3]));
    let (lo, hi) = alphas.iter().fold((255, 0), |(lo, hi), &a| (a.min(lo), a.max(hi)));
    let (mut e0, mut e1) = (quantize(lo), quantize(hi));
    let weights: &[u32] = if index_bits == 2 { &WEIGHTS2 } else { &WEIGHTS3 };
    let level = |e0: u32, e1: u32, w: u32| (expand(e0) * (64 - w) + expand(e1) * w + 32) >> 6;
    let nearest = |a: u32, e0: u32, e1: u32| {
        (0..weights.len() as u32)
            .min_by_key(|&i| level(e0, e1, weights[i as usize]).abs_diff(a))
            .unwrap_or(0)
    };
    let mut indices = alphas.map(|a| nearest(a, e0, e1));
    // the first index has no highest bit: swap the endpoints when it needs one
    let top = weights.len() as u32 - 1;
    if indices[0] > top / 2 {
        (e0, e1) = (e1, e0);
        indices = indices.map(|i| top - i);
    }
    let mut put = |at: u32, n: u32, v: u32| {
        let mask = ((1u128 << n) - 1) << at;
        b = (b & !mask) | ((u128::from(v) << at) & mask);
    };
    put(ends_at, end_bits, e0);
    put(ends_at + end_bits, end_bits, e1);
    for (i, &index) in indices.iter().enumerate() {
        let at = if i == 0 { index_at } else { index_at + i as u32 * index_bits - 1 };
        put(at, if i == 0 { index_bits - 1 } else { index_bits }, index);
    }
    *block = b.to_le_bytes();
    true
}

/// The 16 texels of block (`bx`, `by`), padded with the level's edge as
/// [`encode_level`] pads it. `rgba` holds the whole level ([`encode_chain`]
/// checks its length).
fn source_block(rgba: &[u8], w: usize, h: usize, bx: usize, by: usize) -> [u8; 64] {
    let mut out = [0u8; 64];
    for y in 0..4 {
        let sy = (by * 4 + y).min(h - 1);
        for x in 0..4 {
            let s = (sy * w + (bx * 4 + x).min(w - 1)) * 4;
            out[(y * 4 + x) * 4..][..4].copy_from_slice(&rgba[s..s + 4]);
        }
    }
    out
}

fn ends_kept(src: &[u8; 64], decoded: &[u8; 64]) -> bool {
    src.as_chunks::<4>()
        .0
        .iter()
        .zip(decoded.as_chunks::<4>().0)
        .all(|(s, d)| !alpha_end(s[3]) || d[3] == s[3])
}

/// A mode 6 block (one line, 7-bit endpoints and one shared lowest bit
/// each: bits 63 and 64) with the lowest bits that keep the ends of alpha
/// of `src`; the block is left as it was when no setting does.
fn mend_mode6(block: &mut [u8], src: &[u8; 64]) -> bool {
    if block.len() != 16 || block[0] & 0x7f != 0x40 {
        return false;
    }
    let (b7, b8) = (block[7], block[8]);
    let mut decoded = [0u8; 64];
    for (p0, p1) in [(1, 1), (1, 0), (0, 1), (0, 0)] {
        block[7] = (b7 & 0x7f) | (p0 << 7);
        block[8] = (b8 & 0xfe) | p1;
        bcdec_rs::bc7(block, &mut decoded, 16);
        if ends_kept(src, &decoded) {
            return true;
        }
    }
    (block[7], block[8]) = (b7, b8);
    false
}

/// Blocks of one level; a level that is not made of whole blocks is padded
/// with its edge texels (they only show through filtering at the border).
fn encode_level(settings: &bc7::EncodeSettings, w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let (pw, ph) = (w.next_multiple_of(4), h.next_multiple_of(4));
    if (pw, ph) == (w, h) {
        let surface = RgbaSurface {
            data: &rgba[..w as usize * h as usize * 4],
            width: w,
            height: h,
            stride: w * 4,
        };
        return bc7::compress_blocks(settings, &surface);
    }
    let mut padded = vec![0u8; pw as usize * ph as usize * 4];
    for y in 0..ph as usize {
        let sy = y.min(h as usize - 1);
        for x in 0..pw as usize {
            let sx = x.min(w as usize - 1);
            let (s, d) = ((sy * w as usize + sx) * 4, (y * pw as usize + x) * 4);
            padded[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
        }
    }
    let surface = RgbaSurface {
        data: &padded,
        width: pw,
        height: ph,
        stride: pw * 4,
    };
    bc7::compress_blocks(settings, &surface)
}

/// Decode a bounded cache level for CPU consumers such as appearance hints.
/// Edge blocks may be padded; their padding is never copied to the image.
pub fn decode_level(blocks: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return None;
    }
    let columns = w.div_ceil(4) as usize;
    let rows = h.div_ceil(4) as usize;
    if blocks.len() != columns.checked_mul(rows)?.checked_mul(16)? {
        return None;
    }
    let mut rgba = vec![0; (w as usize).checked_mul(h as usize)?.checked_mul(4)?];
    let mut decoded = [0u8; 64];
    for by in 0..rows {
        for bx in 0..columns {
            let at = (by * columns + bx) * 16;
            bcdec_rs::bc7(&blocks[at..at + 16], &mut decoded, 16);
            let width = (w as usize - bx * 4).min(4) * 4;
            for y in 0..(h as usize - by * 4).min(4) {
                let at = ((by * 4 + y) * w as usize + bx * 4) * 4;
                rgba[at..at + width].copy_from_slice(&decoded[y * 16..y * 16 + width]);
            }
        }
    }
    Some(rgba)
}

/// Whether `blocks` (the BC7 level of `rgba`) differ from it enough to show:
/// more than [`MAX_OFF_PER_MILLE`] of the checked blocks have a texel
/// channel off by more than [`TOLERANCE`]. The whole blocks of the level are
/// looked at (not the padded ones of its edge); a level of fewer than
/// [`MIN_BLOCKS`] is let through: a few texels seen from afar, where one
/// block off would refuse the texture.
pub fn harmed(rgba: &[u8], blocks: &[u8], w: u32, h: u32, alpha: bool) -> bool {
    let (bw, bh) = (w as usize / 4, h as usize / 4);
    if bw * bh < MIN_BLOCKS {
        return false;
    }
    let columns = w.div_ceil(4) as usize;
    let all = bw * bh <= CHECK_ALL_UNDER;
    let (mut checked, mut off) = (0usize, 0usize);
    let mut decoded = [0u8; 64];
    for by in 0..bh {
        for bx in 0..bw {
            // one in four, spread over rows and columns
            if !all && !(bx + 2 * by).is_multiple_of(CHECK_ONE_IN) {
                continue;
            }
            let at = (by * columns + bx) * 16;
            let Some(block) = blocks.get(at..at + 16) else {
                return true;
            };
            bcdec_rs::bc7(block, &mut decoded, 16);
            checked += 1;
            off += usize::from(block_off(rgba, &decoded, w as usize, bx, by, alpha));
        }
    }
    off * 1000 > checked * MAX_OFF_PER_MILLE
}

fn block_off(rgba: &[u8], decoded: &[u8; 64], w: usize, bx: usize, by: usize, alpha: bool) -> bool {
    for y in 0..4 {
        let row = ((by * 4 + y) * w + bx * 4) * 4;
        let Some(src) = rgba.get(row..row + 16) else {
            return true;
        };
        let (src, _) = src.as_chunks::<4>();
        let (decoded, _) = decoded[y * 16..y * 16 + 16].as_chunks::<4>();
        for (s, d) in src.iter().zip(decoded) {
            let shown = if alpha { s[3] as u32 } else { 255 };
            for c in 0..3 {
                if (s[c] as u32).abs_diff(d[c] as u32) * shown > TOLERANCE * 255 {
                    return true;
                }
            }
            if alpha && (s[3] as u32).abs_diff(d[3] as u32) > TOLERANCE {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_mips;

    fn image(w: u32, h: u32, texel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .flat_map(|(x, y)| texel(x, y))
            .collect()
    }

    #[test]
    fn cache_decode_handles_padded_edges_and_rejects_invalid_sizes() {
        let pixels = image(5, 3, |_, _| [210, 120, 60, 255]);
        let blocks = encode_level(&bc7::opaque_fast_settings(), 5, 3, &pixels);
        let rgba = decode_level(&blocks, 5, 3).expect("edge pixels");
        assert_eq!(rgba.len(), 5 * 3 * 4);
        for pixel in rgba.as_chunks::<4>().0 {
            for (actual, expected) in pixel.iter().zip([210u8, 120, 60, 255]) {
                assert!(actual.abs_diff(expected) < 5);
            }
        }
        assert!(decode_level(&blocks[..16], 5, 3).is_none());
        assert!(decode_level(&[], 0, 3).is_none());
        assert!(decode_level(&[], 16385, 3).is_none());
    }

    /// A cheap deterministic noise.
    fn hash(x: u32, y: u32, c: u32) -> u8 {
        let mut v = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ c.wrapping_mul(0xC2B2_AE3D);
        v ^= v >> 15;
        v = v.wrapping_mul(0x2C1B_3C6D);
        (v >> 24) as u8
    }

    #[test]
    fn smooth_textures_compress_to_a_quarter() {
        let rgba = image(64, 32, |x, y| [(x * 4) as u8, (y * 8) as u8, 128, 255]);
        let mips = build_mips(64, 32, rgba, 16);
        let chain = encode_chain(&mips, false).expect("compressible");
        assert_eq!(chain.len(), mips.len());
        for ((w, h, blocks), (mw, mh, _)) in chain.iter().zip(&mips) {
            assert_eq!((w, h), (mw, mh));
            assert_eq!(blocks.len(), w.div_ceil(4) as usize * h.div_ceil(4) as usize * 16);
        }
        assert_eq!(chain[0].2.len() * 4, mips[0].2.len());
        // the levels under one block still decode to their texels
        let (_, _, last) = chain.last().expect("1 × 1 level");
        let mut decoded = [0u8; 64];
        bcdec_rs::bc7(last, &mut decoded, 16);
        let texel = &mips.last().expect("1 × 1 level").2;
        for c in 0..3 {
            assert!(decoded[c].abs_diff(texel[c]) <= 2, "channel {c}");
        }
    }

    #[test]
    fn noise_stays_uncompressed() {
        let rgba = image(64, 64, |x, y| [hash(x, y, 0), hash(x, y, 1), hash(x, y, 2), 255]);
        assert!(encode_chain(&build_mips(64, 64, rgba, 16), false).is_none());
    }

    #[test]
    fn alpha_is_kept_and_hidden_colors_do_not_count() {
        // a soft alpha ramp over a flat color: compressible, alpha preserved
        let rgba = image(32, 32, |x, _| [200, 100, 50, (x * 8) as u8]);
        let mips = build_mips(32, 32, rgba.clone(), 16);
        let chain = encode_chain(&mips, true).expect("compressible");
        assert!(!harmed(&rgba, &chain[0].2, 32, 32, true));
        // noise under fully transparent texels is not seen: still compressible
        let hidden = image(32, 32, |x, y| {
            if x < 16 {
                [hash(x, y, 0), hash(x, y, 1), hash(x, y, 2), 0]
            } else {
                [200, 100, 50, 255]
            }
        });
        assert!(encode_chain(&build_mips(32, 32, hidden, 16), true).is_some());
    }

    /// Every texel at an end of alpha (0 or 255) decodes to it exactly.
    fn ends_exact(mips: &[(u32, u32, Vec<u8>)], chain: &Chain) -> bool {
        mips.iter().zip(chain).all(|((w, h, rgba), (_, _, blocks))| {
            let decoded = decode_level(blocks, *w, *h).expect("level");
            rgba.as_chunks::<4>()
                .0
                .iter()
                .zip(decoded.as_chunks::<4>().0)
                .all(|(s, d)| !alpha_end(s[3]) || s[3] == d[3])
        })
    }

    #[test]
    fn ends_of_alpha_stay_exact() {
        // colored, with opaque texels, transparent ones and soft edges
        // between them in the same blocks (strands of hair, cut-outs): the
        // opaque ones must stay at 255 for a mask cutoff of 255
        let rgba = image(64, 64, |x, y| {
            let a = match (x + y / 2) % 48 {
                n @ 0..=17 => (255 - 15 * n) as u8,
                18..=29 => 0,
                n @ 30..=46 => (15 * (n - 29)) as u8,
                _ => 255,
            };
            [(x * 3 + y) as u8, (y * 4) as u8, (130 + x) as u8, a]
        });
        let mips = build_mips(64, 64, rgba, 16);
        let chain = encode_chain(&mips, true).expect("compressible");
        assert!(ends_exact(&mips, &chain));
        // opaque everywhere but one corner, under a busy color: mode 6 alone
        // leaves its opaque texels at 254 in some blocks
        let rgba = image(64, 64, |x, y| {
            [
                hash(x / 2, y / 2, 0) / 4 + (x * 3) as u8,
                (y * 3) as u8,
                90,
                if x + y < 4 { 0 } else { 255 },
            ]
        });
        let mips = build_mips(64, 64, rgba, 16);
        let chain = encode_chain(&mips, true).expect("compressible");
        assert!(ends_exact(&mips, &chain));
    }

    #[test]
    fn snapped_alpha_keeps_its_ends_and_the_color() {
        let src: [u8; 64] = std::array::from_fn(|i| match i % 4 {
            3 => [255, 0, 200, 37, 255, 255, 0, 120, 255, 64, 0, 255, 180, 0, 255, 9][i / 4],
            c => (40 * c + i) as u8,
        });
        for settings in [
            // mode 5, then mode 4 with either index selection
            bc7::EncodeSettings {
                mode_selection: [false, false, true, false],
                ..bc7::alpha_ultra_fast_settings()
            },
            bc7::EncodeSettings {
                mode_selection: [false, false, true, false],
                mode45_channel0: 0,
                ..bc7::alpha_slow_settings()
            },
        ] {
            let surface = RgbaSurface {
                data: &src,
                width: 4,
                height: 4,
                stride: 16,
            };
            let encoded = bc7::compress_blocks(&settings, &surface);
            let mut block: [u8; 16] = encoded[..16].try_into().expect("one block");
            let mut before = [0u8; 64];
            bcdec_rs::bc7(&block, &mut before, 16);
            if !snap_alpha(&mut block, &src) {
                // (rotated alpha: left to the encoder)
                continue;
            }
            let mut after = [0u8; 64];
            bcdec_rs::bc7(&block, &mut after, 16);
            assert!(
                ends_kept(&src, &after),
                "{:?}",
                after.as_chunks::<4>().0.iter().map(|t| t[3]).collect::<Vec<_>>()
            );
            for (s, d) in src.as_chunks::<4>().0.iter().zip(after.as_chunks::<4>().0) {
                assert!(s[3].abs_diff(d[3]) <= 20, "alpha {} for {}", d[3], s[3]);
            }
            for (b, a) in before.as_chunks::<4>().0.iter().zip(after.as_chunks::<4>().0) {
                assert_eq!(b[..3], a[..3], "the color does not move");
            }
        }
    }

    #[test]
    fn mode6_endpoint_bits_mend_opaque_blocks() {
        // an opaque block whose endpoints' lowest bits are 0 decodes at 254
        let src: [u8; 64] = std::array::from_fn(|i| if i % 4 == 3 { 255 } else { (i * 7) as u8 });
        let surface = RgbaSurface {
            data: &src,
            width: 4,
            height: 4,
            stride: 16,
        };
        let settings = bc7::EncodeSettings {
            channels: 4,
            ..bc7::opaque_ultra_fast_settings()
        };
        let mut block = bc7::compress_blocks(&settings, &surface);
        assert_eq!(block[0] & 0x7f, 0x40, "mode 6");
        (block[7], block[8]) = (block[7] & 0x7f, block[8] & 0xfe);
        let mut decoded = [0u8; 64];
        bcdec_rs::bc7(&block, &mut decoded, 16);
        assert!(!ends_kept(&src, &decoded));
        assert!(mend_mode6(&mut block, &src));
        bcdec_rs::bc7(&block, &mut decoded, 16);
        assert!(ends_kept(&src, &decoded));
    }

    #[test]
    fn sizes_that_are_not_whole_blocks_are_refused() {
        let rgba = image(30, 32, |_, _| [9, 9, 9, 255]);
        assert!(encode_chain(&build_mips(30, 32, rgba, 16), false).is_none());
        assert!(encode_chain(&[], false).is_none());
        // a short level
        assert!(encode_chain(&[(8, 8, vec![0; 100])], false).is_none());
    }

    #[test]
    fn levels_between_blocks_are_padded_with_their_edge() {
        // 12 × 20: the next levels are 6 × 10 and 3 × 5
        let rgba = image(12, 20, |x, _| [(x * 8) as u8, (x * 4) as u8, 77, 255]);
        let mips = build_mips(12, 20, rgba, 16);
        let chain = encode_chain(&mips, false).expect("compressible");
        assert_eq!(chain[1].2.len(), 2 * 3 * 16);
        assert_eq!(chain[2].2.len(), 2 * 16);
        // the first block of level 1 holds its texels, not the padding
        let mut decoded = [0u8; 64];
        bcdec_rs::bc7(&chain[1].2[..16], &mut decoded, 16);
        let src = &mips[1].2;
        for y in 0..4 {
            for x in 0..4 {
                for c in 0..3 {
                    let (d, s) = (decoded[(y * 4 + x) * 4 + c], src[(y * 6 + x) * 4 + c]);
                    assert!(d.abs_diff(s) <= 8, "texel {x},{y} channel {c}: {d} for {s}");
                }
            }
        }
    }
}
