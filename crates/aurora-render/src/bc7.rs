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
        let blocks = encode_level(settings, *w, *h, rgba);
        if harmed(rgba, &blocks, *w, *h, alpha) {
            return None;
        }
        out.push((*w, *h, blocks));
    }
    Some(out)
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
