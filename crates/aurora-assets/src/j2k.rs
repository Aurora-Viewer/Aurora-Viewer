//! JPEG2000 texture decoding.
//!
//! Ported from the Second Life / Firestorm viewer sources (originally LGPL 2.1):
//! - `indra/llimage/llimagej2c.cpp` (`LLImageJ2C::calcDataSizeJ2C`, `calcHeaderSizeJ2C`)
//! - `indra/llimagej2coj/llimagej2coj.cpp` (`JPEG2KDecode`, memory stream callbacks,
//!   `LLImageJ2COJ::decodeImpl`)
//!
//! Copyright (C) 2006-2024, Linden Research, Inc. and the Firestorm project.
//!
//! Differences from the C++ code:
//! - Output rows are stored **top-to-bottom** in codestream order. `LLImageRaw`
//!   stores rows bottom-up (GL convention); a renderer with a top-left texture
//!   origin (wgpu/D3D/Metal) can upload [`DecodedImage::data`] as-is and sample
//!   SL texture coordinates with `v' = 1 - v`.
//! - Components with precision != 8, signed samples and subsampled components
//!   are converted to 8-bit full-resolution samples (LL assumes 8-bit unsigned).
//! - At most 4 components are returned (LL's `max_channel_count`).

use std::ffi::{CStr, c_char, c_void};
use std::ptr;
use std::time::{Duration, Instant};

use openjpeg_sys as opj;

use crate::AssetError;
mod encode;
pub use encode::{encode_texture_j2k, encode_thumbnail_j2k};

/// `MAX_DISCARD_LEVEL` from `llimage.h`.
pub const MAX_DISCARD_LEVEL: u8 = 5;
/// `FIRST_PACKET_SIZE` from `llimage.h`, returned by `LLImageJ2C::calcHeaderSizeJ2C`.
pub const HEADER_SIZE_ESTIMATE: usize = 600;
/// `MAX_BLOCK_SIZE` from `llimage.h`.
const MAX_BLOCK_SIZE: i64 = 64;
/// `DEFAULT_COMPRESSION_RATE` from `llimagej2c.h`.
const DEFAULT_COMPRESSION_RATE: f32 = 1.0 / 8.0;

/// Sanity limits applied before handing a codestream to OpenJPEG, so that a
/// malicious header cannot request gigantic allocations.
const MAX_DIMENSION: u32 = 16384;
const MAX_PIXELS: u64 = 8192 * 8192;
const MAX_HEADER_COMPONENTS: u16 = 16;

const JP2_SIGNATURE: [u8; 12] = [0, 0, 0, 0x0C, b'j', b'P', b' ', b' ', 0x0D, 0x0A, 0x87, 0x0A];

/// Basic codestream parameters read from the SIZ/COD marker segments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct J2kInfo {
    pub width: u32,
    pub height: u32,
    pub components: u8,
    /// Number of wavelet decomposition levels (COD `SPcod` NL); the maximum
    /// usable discard level (`cp_reduce`).
    pub levels: u8,
}

/// A decoded image: tightly packed 8-bit samples, `components` bytes per pixel,
/// rows top-to-bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub components: u8,
    pub data: Vec<u8>,
    /// Resolution reduction actually applied (each level halves both dimensions).
    pub discard: u8,
}

/// Locate the raw J2K codestream: either the data itself (SL textures) or the
/// contents of the `jp2c` box of a JP2 file.
fn locate_codestream(data: &[u8]) -> Option<&[u8]> {
    if data.starts_with(&[0xFF, 0x4F]) {
        return Some(data);
    }
    if !data.starts_with(&JP2_SIGNATURE) {
        return None;
    }
    let mut pos = 0usize;
    loop {
        let hdr = data.get(pos..pos.checked_add(8)?)?;
        let lbox = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as u64;
        let tbox = &hdr[4..8];
        let (header_len, box_len) = match lbox {
            0 => (8u64, (data.len() - pos) as u64),
            1 => {
                let x = data.get(pos + 8..pos.checked_add(16)?)?;
                let mut b = [0u8; 8];
                b.copy_from_slice(x);
                (16u64, u64::from_be_bytes(b))
            }
            n => (8u64, n),
        };
        if box_len < header_len {
            return None;
        }
        let content_start = pos.checked_add(header_len as usize)?;
        if tbox == b"jp2c" {
            let end = pos.saturating_add(usize::try_from(box_len).unwrap_or(usize::MAX)).min(data.len());
            return data.get(content_start..end);
        }
        pos = pos.checked_add(usize::try_from(box_len).ok()?)?;
        if pos >= data.len() {
            return None;
        }
    }
}

fn be16(d: &[u8], at: usize) -> Option<u16> {
    let b = d.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([b[0], b[1]]))
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    let b = d.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// Parse SIZ and COD from a raw codestream main header. The header is
/// walked by [`walk_header`], the same walk as [`parse_layout`] and as
/// OpenJPEG's `opj_j2k_read_header_procedure` (unknown markers passed two
/// bytes at a time, up to the first SOT), so every reader of the header
/// sees the same segments. The first SIZ counts (OpenJPEG refuses a SIZ
/// anywhere else), and the last COD (each one replaces the default coding
/// style, `opj_j2k_read_cod`).
fn parse_main_header(cs: &[u8]) -> Option<J2kInfo> {
    if be16(cs, 0)? != 0xFF4F {
        return None;
    }
    let mut steps = cs.len() + 4096;
    let mut size: Option<Option<(u32, u32, u16)>> = None;
    let mut levels: Option<u8> = None;
    walk_header(
        cs,
        2,
        |m| m == 0xFF90,
        &mut steps,
        |marker, seg| match marker {
            0xFF51 if size.is_none() => {
                // SIZ: Rsiz(2) Xsiz Ysiz XOsiz YOsiz XTsiz YTsiz XTOsiz YTOsiz (4 each) Csiz(2)
                size = Some((|| {
                    let w = be32(seg, 2)?.checked_sub(be32(seg, 10)?)?;
                    let h = be32(seg, 6)?.checked_sub(be32(seg, 14)?)?;
                    let csiz = be16(seg, 34)?;
                    (w != 0 && h != 0 && csiz != 0).then_some((w, h, csiz))
                })());
            }
            // COD: Scod(1) prog(1) layers(2) mct(1) NL(1) ...
            0xFF52 => levels = seg.get(5).copied().or(levels),
            _ => {}
        },
    )?;
    let (width, height, comps) = size??;
    Some(J2kInfo {
        width,
        height,
        components: comps.min(255) as u8,
        levels: levels?,
    })
}

/// OpenJPEG allocates a tile-part's declared length (SOT `Psot`, up to 4 GB)
/// before reading it, whatever the stream holds: found by fuzzing, a 163-byte
/// texture made it ask for 1 GB. A tile-part that claims more than the rest
/// of the codestream and more than `plausible` (a partial download is cut
/// short on purpose: its tile-parts keep their length) gets `Psot = 0`, the
/// standard "until the end of the codestream" (ISO 15444-1 A.4.2). Every SOT
/// marker of the codestream is checked, not only those the header's segment
/// lengths lead to: OpenJPEG skips unknown bytes until it finds the next
/// marker, and `FF 90` never appears inside coded data (A.1). Returns a
/// patched copy only when one is needed.
fn bound_tile_parts(cs: &[u8], plausible: usize) -> Option<Vec<u8>> {
    let mut copy: Option<Vec<u8>> = None;
    // SOT, Lsot = 10 (the only length OpenJPEG accepts), Isot, Psot.
    for pos in 0..cs.len().saturating_sub(10) {
        if cs[pos..pos + 4] != [0xFF, 0x90, 0x00, 0x0A] {
            continue;
        }
        let psot = be32(cs, pos + 6)? as usize;
        if psot > cs.len() - pos && psot > plausible {
            copy.get_or_insert_with(|| cs.to_vec())[pos + 6..pos + 10].fill(0);
        }
    }
    copy
}

/// OpenJPEG 2.5.3 sizes a tile's tile-part index from the TLM marker segments
/// of the main header, then fills one slot per SOT as it reads the codestream.
/// On the branch where a SOT gives no tile-part count (TNsot = 0) it does not
/// check the tile-part number (TPsot) against that size, so a later
/// `opj_j2k_read_sod` can write one slot past the index (a heap overflow;
/// fixed on OpenJPEG master by PR #1621, not yet in any release, hence
/// openjpeg-sys 1.0.12 is still affected). Aurora always decodes a whole
/// texture in codestream order and never uses TLM to seek, so every TLM
/// segment of the main header is disabled before OpenJPEG reads it: its Stlm
/// byte is set to the reserved size code (ST = 3), which `opj_j2k_read_tlm`
/// rejects as invalid and OpenJPEG then falls back to growing the index from
/// the SOT markers, the same as a stream with no TLM. Byte offsets are kept,
/// so no other field moves. Returns a patched copy only when a TLM is present.
///
/// The TLM marker (0xFF55) lives only in the main header, which runs from the
/// SOC to the first SOT (ISO 15444-1 A.4.4). The header is walked by
/// [`walk_header`], the way `opj_j2k_read_header_procedure` reads it: an
/// unknown marker (SOD and EOC included, which OpenJPEG does not know in a
/// header) is passed two bytes at a time, as `opj_j2k_read_unk` looks for the
/// next known marker, so no byte sequence can hide a TLM that OpenJPEG would
/// still reach. A 0xFF55 pair inside a segment's body or in later coded data
/// is never touched. The walk fails closed: a main header it cannot follow
/// to its first SOT (a segment too short or cut, a TLM with no Stlm byte) is
/// refused, as OpenJPEG would refuse it anyway.
fn disable_tlm_index(cs: &[u8]) -> Result<Option<Vec<u8>>, AssetError> {
    const REFUSED: AssetError = AssetError::J2k("JPEG2000 main header");
    let mut steps = cs.len() + 16;
    let reached_sot = std::cell::Cell::new(false);
    let mut stlm_at: Vec<usize> = Vec::new();
    let mut tlm_cut = false;
    let stop = |m: u16| {
        let sot = m == 0xFF90;
        reached_sot.set(sot);
        sot
    };
    // past the SOC marker (0xFF4F)
    walk_header(cs, 2, stop, &mut steps, |marker, body| {
        if marker == 0xFF55 {
            // TLM body: Ztlm(1) Stlm(1) …; `body` is a slice of `cs`.
            match body.get(1) {
                Some(stlm) => stlm_at.push(stlm as *const u8 as usize - cs.as_ptr() as usize),
                None => tlm_cut = true,
            }
        }
    })
    .ok_or(REFUSED)?;
    if !reached_sot.get() || tlm_cut {
        return Err(REFUSED);
    }
    if stlm_at.is_empty() {
        return Ok(None);
    }
    let mut copy = cs.to_vec();
    for at in stlm_at {
        // Force ST = 3 in Stlm (bits 4-5).
        copy[at] |= 0x30;
    }
    Ok(Some(copy))
}

/// What OpenJPEG may hold at once for one decode, estimated from the headers
/// ([`decode_memory`]) before it reads them: its header parsing keeps the
/// coding parameters of every tile, each tile it decodes the structures of
/// all its code-blocks and precincts whatever the resolution asked for, and
/// the samples are 32-bit for every component. Headers can declare all of
/// these far beyond any texture's needs. An SL texture (one tile,
/// at most 2048² and 4 components) needs about 85 MB at full resolution, a
/// 4096² one with 4 components about 330 MB.
const DECODE_BUDGET: u64 = 512 << 20;
/// Tiles of a codestream: SL textures have one. OpenJPEG itself accepts 65,535.
const MAX_TILES: u64 = 4096;
// Sizes of OpenJPEG 2.5's allocations, rounded up.
/// Coding parameters of one tile (`opj_tcp_t`, its 32 progression orders) and its index.
const TILE_BYTES: u64 = 8 << 10;
/// Per component of each tile (`opj_tccp_t`, its 97 quantization step sizes).
const TILE_COMPONENT_BYTES: u64 = 2 << 10;
/// One code-block: `opj_tcd_cblk_dec_t`, its first 10 segments, its tag tree leaves.
const CODEBLOCK_BYTES: u64 = 512;
/// One precinct of one band: `opj_tcd_precinct_t` and its two tag trees.
const PRECINCT_BYTES: u64 = 256;

/// The SIZ fields that size OpenJPEG's allocations (reference grid coordinates).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Siz {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    tile_x0: u32,
    tile_y0: u32,
    tile_w: u32,
    tile_h: u32,
    /// Subsampling (XRsiz, YRsiz) of each component.
    sub: Vec<(u8, u8)>,
}

fn parse_siz(body: &[u8]) -> Option<Siz> {
    // Rsiz(2) Xsiz Ysiz XOsiz YOsiz XTsiz YTsiz XTOsiz YTOsiz (4 each) Csiz(2),
    // then Ssiz XRsiz YRsiz (1 each) per component.
    let csiz = usize::from(be16(body, 34)?);
    let sub = (0..csiz)
        .map(|i| Some((*body.get(37 + 3 * i)?, *body.get(38 + 3 * i)?)))
        .collect::<Option<Vec<_>>>()?;
    Some(Siz {
        x1: be32(body, 2)?,
        y1: be32(body, 6)?,
        x0: be32(body, 10)?,
        y0: be32(body, 14)?,
        tile_w: be32(body, 18)?,
        tile_h: be32(body, 22)?,
        tile_x0: be32(body, 26)?,
        tile_y0: be32(body, 30)?,
        sub,
    })
}

/// The costliest coding style among all the COD and COC segments of a
/// codestream (main and tile-part headers): most decomposition levels and
/// layers, smallest code-blocks and precincts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Coding {
    levels: u8,
    layers: u16,
    /// log2 of the code-block width and height.
    block: (u8, u8),
    /// log2 of the precinct width and height.
    precinct: (u8, u8),
}

impl Coding {
    /// Before any segment: each one only makes it costlier.
    const NONE: Coding = Coding {
        levels: 0,
        layers: 1,
        block: (10, 10),
        precinct: (15, 15),
    };

    /// SPcod / SPcoc at `at`: NL, xcb, ycb, style, transform, then one
    /// precinct size per resolution when `precincts` is set. Values OpenJPEG
    /// refuses are clamped to what it accepts.
    fn add_style(&mut self, body: &[u8], at: usize, precincts: bool) {
        let (Some(&nl), Some(&xcb), Some(&ycb)) = (body.get(at), body.get(at + 1), body.get(at + 2)) else {
            return;
        };
        self.levels = self.levels.max(nl.min(32));
        self.block = (self.block.0.min(xcb.min(8) + 2), self.block.1.min(ycb.min(8) + 2));
        if precincts {
            for &b in body.iter().skip(at + 5).take(usize::from(nl) + 1) {
                self.precinct = (self.precinct.0.min(b & 0x0F), self.precinct.1.min(b >> 4));
            }
        }
    }

    /// COD: Scod(1) progression(1) layers(2) mct(1) SPcod.
    fn add_cod(&mut self, body: &[u8]) {
        let Some(&scod) = body.first() else { return };
        if let Some(layers) = be16(body, 2) {
            self.layers = self.layers.max(layers);
        }
        self.add_style(body, 5, scod & 1 != 0);
    }

    /// COC: Ccoc(1, or 2 from 257 components) Scoc(1) SPcoc.
    fn add_coc(&mut self, body: &[u8], components: usize) {
        let ccoc = if components < 257 { 1 } else { 2 };
        let Some(&scoc) = body.get(ccoc) else { return };
        self.add_style(body, ccoc + 1, scoc & 1 != 0);
    }
}

/// The marker segments OpenJPEG reads in a main or tile-part header
/// (`j2k_memory_marker_handler_tab`), all with a length.
fn is_header_segment(marker: u16) -> bool {
    matches!(
        marker,
        0xFF50..=0xFF53 | 0xFF55 | 0xFF57..=0xFF59 | 0xFF5C..=0xFF61 | 0xFF63 | 0xFF64 | 0xFF74 | 0xFF75 | 0xFF77 | 0xFF78
    )
}

/// Walk the marker segments of a header from `pos` as OpenJPEG reads them,
/// handing each one (its position, marker, contents) to `segment`, up to a marker `stop`
/// accepts or the end of the data. Other bytes are passed two at a time, as
/// `opj_j2k_read_unk` looks for the next known marker. `steps` bounds the
/// work over all the headers of a codestream: `None` once it runs out.
fn walk_header(
    cs: &[u8],
    mut pos: usize,
    stop: impl Fn(u16) -> bool,
    steps: &mut usize,
    mut segment: impl FnMut(u16, &[u8]),
) -> Option<()> {
    while let Some(marker) = be16(cs, pos) {
        *steps = steps.checked_sub(1)?;
        if stop(marker) {
            break;
        }
        if !is_header_segment(marker) {
            pos += 2;
            continue;
        }
        // OpenJPEG stops on a segment that is too short or cut (a partial
        // download can end anywhere).
        let Some(len) = be16(cs, pos + 2).map(usize::from).filter(|&l| l >= 2) else {
            break;
        };
        let Some(body) = cs.get(pos + 4..pos + 2 + len) else {
            break;
        };
        segment(marker, body);
        pos += 2 + len;
    }
    Some(())
}

/// The SIZ of the main header and the costliest coding style of the
/// codestream. Every tile-part header is read, found like in
/// [`bound_tile_parts`] by its SOT marker: a COD or COC there can change the
/// code-blocks, precincts, levels and layers of its tile.
fn parse_layout(cs: &[u8]) -> Option<(Siz, Coding)> {
    let mut steps = cs.len() + 4096;
    let mut siz: Option<Siz> = None;
    let mut siz_seen = false;
    let mut coding = Coding::NONE;
    // Only the first SIZ counts, as in `parse_main_header`.
    let mut visit = |marker: u16, body: &[u8]| match marker {
        0xFF51 if !siz_seen => {
            siz_seen = true;
            siz = parse_siz(body);
        }
        0xFF52 => coding.add_cod(body),
        0xFF53 => {
            if let Some(s) = &siz {
                coding.add_coc(body, s.sub.len());
            }
        }
        _ => {}
    };
    if be16(cs, 0)? != 0xFF4F {
        return None;
    }
    walk_header(cs, 2, |m| m == 0xFF90, &mut steps, &mut visit)?;
    for pos in 0..cs.len().saturating_sub(10) {
        if cs[pos..pos + 4] == [0xFF, 0x90, 0x00, 0x0A] {
            // SOT is 12 bytes; its header ends at SOD (or another SOT, or EOC).
            walk_header(cs, pos + 12, |m| matches!(m, 0xFF90 | 0xFF93 | 0xFFD9), &mut steps, &mut visit)?;
        }
    }
    Some((siz?, coding))
}

/// Tiles across and down, as `opj_j2k_read_siz` counts them: from the tile
/// grid origin, which must lie in the first tile, to the image's far edge.
fn tile_grid(siz: &Siz) -> Option<(u64, u64)> {
    let first_tile_holds_origin = |t0: u32, size: u32, x0: u32| t0 <= x0 && u64::from(t0) + u64::from(size) > u64::from(x0);
    if siz.tile_w == 0
        || siz.tile_h == 0
        || siz.x1 <= siz.x0
        || siz.y1 <= siz.y0
        || !first_tile_holds_origin(siz.tile_x0, siz.tile_w, siz.x0)
        || !first_tile_holds_origin(siz.tile_y0, siz.tile_h, siz.y0)
    {
        return None;
    }
    let across = u64::from(siz.x1 - siz.tile_x0).div_ceil(u64::from(siz.tile_w));
    let down = u64::from(siz.y1 - siz.tile_y0).div_ceil(u64::from(siz.tile_h));
    Some((across, down))
}

/// Size of `n` samples after `levels` halvings, plus one for the alignment.
fn shrink(n: u64, levels: u32) -> u64 {
    n.div_ceil(1u64 << levels.min(63)).saturating_add(1)
}

/// Cells of `2^exp` samples that cover `len` samples, wherever they start.
fn cells(len: u64, exp: u8) -> u64 {
    len.saturating_add(1).div_ceil(1u64 << exp.min(63)).saturating_add(1)
}

/// Upper estimate, in bytes, of what OpenJPEG holds at once to decode the
/// codestream at resolution reduction `reduce` (the Legacy Viewer's
/// LLImageJ2COJ::decodeImpl has no such bound). OpenJPEG decodes the
/// tiles one after the other, reusing the code-block and precinct structures
/// of the previous one: those count for the largest tile.
fn decode_memory(siz: &Siz, coding: &Coding, reduce: u8, tiles: (u64, u64)) -> u64 {
    let tile_count = tiles.0.saturating_mul(tiles.1);
    let comps = siz.sub.len() as u64;
    let levels = u32::from(coding.levels.min(32));
    let reduce = u32::from(reduce).min(levels);
    let (w, h) = (u64::from(siz.x1 - siz.x0), u64::from(siz.y1 - siz.y0));
    let (px, py) = coding.precinct;
    let (bx, by) = coding.block;

    let mut total = tile_count.saturating_mul(TILE_BYTES + comps * TILE_COMPONENT_BYTES);
    let mut tile_precincts = 1u64;
    for &(dx, dy) in &siz.sub {
        let (dx, dy) = (u64::from(dx.max(1)), u64::from(dy.max(1)));
        let cw = u64::from(siz.x1).div_ceil(dx) - u64::from(siz.x0).div_ceil(dx);
        let ch = u64::from(siz.y1).div_ceil(dy) - u64::from(siz.y0).div_ceil(dy);
        // The largest tile, within this component.
        let tw = u64::from(siz.tile_w).min(w).div_ceil(dx) + 1;
        let th = u64::from(siz.tile_h).min(h).div_ceil(dy) + 1;
        // 32-bit samples: the image, and one tile besides when there are several.
        let mut samples = shrink(cw, reduce).saturating_mul(shrink(ch, reduce));
        if tile_count > 1 {
            samples = samples.saturating_add(shrink(tw, reduce).saturating_mul(shrink(th, reduce)));
        }
        total = total.saturating_add(samples.saturating_mul(4));
        // Every resolution of the tile, the reduced ones included.
        for res in 0..=levels {
            let (rw, rh) = (shrink(tw, levels - res), shrink(th, levels - res));
            let precincts = cells(rw, px).saturating_mul(cells(rh, py));
            tile_precincts = tile_precincts.max(precincts);
            // Resolution 0 has one band (LL); the others three, each half as
            // large, with precincts of half the size (B.6).
            let (bands, blocks) = if res == 0 {
                (1, cells(rw, bx.min(px)).saturating_mul(cells(rh, by.min(py))))
            } else {
                let (bw, bh) = (rw.div_ceil(2), rh.div_ceil(2));
                let (sx, sy) = (bx.min(px.saturating_sub(1)), by.min(py.saturating_sub(1)));
                (3, cells(bw, sx).saturating_mul(cells(bh, sy)))
            };
            let bytes = precincts
                .saturating_mul(PRECINCT_BYTES)
                .saturating_add(blocks.saturating_mul(CODEBLOCK_BYTES));
            total = total.saturating_add(bytes.saturating_mul(bands));
        }
    }
    // The packet iterator's inclusion flags (`opj_pi_create_decode`): an
    // OPJ_INT16 per layer + 1, resolution, component and precinct of a tile.
    let include = (u64::from(coding.layers) + 1)
        .saturating_mul(u64::from(levels) + 1)
        .saturating_mul(comps)
        .saturating_mul(tile_precincts)
        .saturating_mul(2);
    // The 8-bit pixels `convert_image` returns.
    let pixels = shrink(w, reduce).saturating_mul(shrink(h, reduce)).saturating_mul(comps.min(4));
    total.saturating_add(include).saturating_add(pixels)
}

/// Refuse a codestream whose decode at `reduce` would need more than
/// [`DECODE_BUDGET`] or has more than [`MAX_TILES`] tiles. Returns the SIZ,
/// which [`convert_image`] checks the decoded components against.
fn check_decode_memory(cs: &[u8], reduce: u8) -> Result<Siz, AssetError> {
    let (siz, coding) = parse_layout(cs).ok_or(AssetError::J2k("JPEG2000 header layout"))?;
    let tiles = tile_grid(&siz).ok_or(AssetError::J2k("JPEG2000 tile grid"))?;
    if tiles.0.saturating_mul(tiles.1) > MAX_TILES {
        return Err(AssetError::TooLarge("JPEG2000 tile count"));
    }
    if decode_memory(&siz, &coding, reduce, tiles) > DECODE_BUDGET {
        return Err(AssetError::TooLarge("JPEG2000 decoder memory"));
    }
    Ok(siz)
}

/// Read image dimensions, component count and decomposition levels from the
/// codestream main header without decoding. Returns `None` if the header is
/// missing, incomplete (SIZ and COD must both be present) or malformed.
pub fn j2k_info(data: &[u8]) -> Option<J2kInfo> {
    parse_main_header(locate_codestream(data)?)
}

/// Port of `LLImageJ2C::calcDataSizeJ2C` (Firestorm FIRE-35987 variant, rate
/// `DEFAULT_COMPRESSION_RATE`): estimated number of bytes, including the
/// `FIRST_PACKET_SIZE` (600 byte) header margin, needed to decode `discard`.
/// `discard` is clamped to `0..=MAX_DISCARD_LEVEL` like `LLImageJ2C::calcDataSize`.
pub fn bytes_for_discard(info: &J2kInfo, discard: u8) -> usize {
    calc_data_size_j2c(
        info.width as i64,
        info.height as i64,
        discard.min(MAX_DISCARD_LEVEL) as i64,
        DEFAULT_COMPRESSION_RATE,
    )
}

fn calc_data_size_j2c(w: i64, h: i64, discard_level: i64, rate: f32) -> usize {
    const PRECISION: i64 = 8;
    const MAX_COMPONENTS: i64 = 4;
    const HARD_CAP: i64 = 12;
    const BASE_LAYER_AREA: i64 = MAX_BLOCK_SIZE * MAX_BLOCK_SIZE;
    const BITS_PER_TILE: i64 = MAX_COMPONENTS * PRECISION;

    let discard_layers = (5 - discard_level).max(0);
    let rate64 = rate as f64;
    let scaled_bits = |layer_area: i64| -> i64 { ((layer_area * BITS_PER_TILE) as f64 * rate64).round() as i64 };

    let total_bits = if w <= 0 || h <= 0 {
        scaled_bits(BASE_LAYER_AREA)
    } else {
        // A header can claim 2^32 - 1 on both sides: the product overflows.
        let surface = w.saturating_mul(h);
        let mut layer_area = BASE_LAYER_AREA;
        let mut nb_layers: i64 = 1;
        let mut total = scaled_bits(layer_area);
        while surface > layer_area && nb_layers < HARD_CAP {
            if nb_layers <= discard_layers {
                total += scaled_bits(layer_area);
            }
            nb_layers += 1;
            layer_area *= 4;
        }
        let max_dimension = w.max(h);
        let ratio = max_dimension as f32 / MAX_BLOCK_SIZE as f32;
        let dimension_layers = if ratio > 0.0 {
            ((ratio.log2().floor() as i64) + 1).max(1)
        } else {
            1
        };
        if dimension_layers > nb_layers {
            let mut extra = dimension_layers.min(HARD_CAP) - nb_layers;
            while extra > 0 {
                extra -= 1;
                if nb_layers <= discard_layers {
                    total += scaled_bits(layer_area);
                }
                nb_layers += 1;
                layer_area *= 4;
            }
        }
        total
    };
    let est = total_bits / 8 + HEADER_SIZE_ESTIMATE as i64;
    est.clamp(0, i32::MAX as i64) as usize
}

// ---------------------------------------------------------------------------
// OpenJPEG FFI wrapper
// ---------------------------------------------------------------------------

/// In-memory input stream state (port of `JPEG2KBase` + `opj_read/skip/seek`).
struct MemStream<'a> {
    data: &'a [u8],
    offset: usize,
}

unsafe extern "C" fn mem_read(buffer: *mut c_void, nb_bytes: usize, user: *mut c_void) -> usize {
    if user.is_null() || buffer.is_null() {
        return usize::MAX;
    }
    // SAFETY: `user` is the `MemStream` registered with `opj_stream_set_user_data`;
    // it lives on the stack of `decode_j2k` for longer than the stream and is only
    // accessed from this (single) decoding thread.
    let s = unsafe { &mut *(user as *mut MemStream<'_>) };
    if s.offset >= s.data.len() {
        s.offset = s.data.len();
        return usize::MAX; // (OPJ_SIZE_T)-1: end of stream
    }
    let to_read = nb_bytes.min(s.data.len() - s.offset);
    // SAFETY: OpenJPEG guarantees `buffer` is writable for `nb_bytes` bytes, and
    // `to_read <= nb_bytes`; the source range is in bounds of `s.data` by the check above.
    unsafe {
        ptr::copy_nonoverlapping(s.data.as_ptr().add(s.offset), buffer as *mut u8, to_read);
    }
    s.offset += to_read;
    to_read
}

unsafe extern "C" fn mem_skip(nb_bytes: i64, user: *mut c_void) -> i64 {
    if user.is_null() {
        return -1;
    }
    // SAFETY: see `mem_read`.
    let s = unsafe { &mut *(user as *mut MemStream<'_>) };
    let len = s.data.len() as i64;
    let new_offset = (s.offset as i64).saturating_add(nb_bytes);
    if new_offset < 0 || new_offset > len {
        s.offset = new_offset.clamp(0, len) as usize;
        return -1;
    }
    s.offset = new_offset as usize;
    nb_bytes
}

unsafe extern "C" fn mem_seek(offset: i64, user: *mut c_void) -> opj::OPJ_BOOL {
    if user.is_null() {
        return 0;
    }
    // SAFETY: see `mem_read`.
    let s = unsafe { &mut *(user as *mut MemStream<'_>) };
    if offset < 0 || offset > s.data.len() as i64 {
        return 0;
    }
    s.offset = offset as usize;
    1
}

fn log_opj_message(level: log::Level, msg: *const c_char) {
    if msg.is_null() || !log::log_enabled!(target: "aurora_assets::j2k", level) {
        return;
    }
    // SAFETY: OpenJPEG passes a valid NUL-terminated C string that outlives the callback.
    let text = unsafe { CStr::from_ptr(msg) };
    log::log!(target: "aurora_assets::j2k", level, "OpenJPEG: {}", text.to_string_lossy().trim_end());
}

unsafe extern "C" fn opj_error_cb(msg: *const c_char, _: *mut c_void) {
    log_opj_message(log::Level::Debug, msg);
}

unsafe extern "C" fn opj_warning_cb(msg: *const c_char, _: *mut c_void) {
    log_opj_message(log::Level::Trace, msg);
}

unsafe extern "C" fn opj_info_cb(msg: *const c_char, _: *mut c_void) {
    log_opj_message(log::Level::Trace, msg);
}

struct StreamGuard(*mut opj::opj_stream_t);
impl Drop for StreamGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer came from `opj_stream_create` and is destroyed exactly once.
            unsafe { opj::opj_stream_destroy(self.0) }
        }
    }
}

struct CodecGuard(*mut opj::opj_codec_t);
impl Drop for CodecGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer came from `opj_create_decompress` and is destroyed exactly once.
            unsafe { opj::opj_destroy_codec(self.0) }
        }
    }
}

struct ImageGuard(*mut opj::opj_image_t);
impl Drop for ImageGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the image was allocated by OpenJPEG (`opj_read_header`) and is
            // destroyed exactly once, after which nothing references it.
            unsafe { opj::opj_image_destroy(self.0) }
        }
    }
}

/// Decode a J2K codestream (or JP2 file) at the given discard level.
///
/// `discard` is used as OpenJPEG `cp_reduce` and is clamped to the number of
/// decomposition levels. Truncated codestreams (partial HTTP range fetches)
/// are decoded as far as possible (non-strict mode). Each call uses its own
/// codec instance, so this is safe to call concurrently.
pub fn decode_j2k(data: &[u8], discard: u8) -> Result<DecodedImage, AssetError> {
    decode_j2k_timed(data, discard).map(|(img, _)| img)
}

/// Where the time of one decode went ([`decode_j2k_timed`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DecodeTimings {
    /// OpenJPEG: header, entropy decoding, inverse wavelet and colour transforms.
    pub codec: Duration,
    /// Planar 32-bit samples to interleaved 8-bit pixels.
    pub convert: Duration,
}

/// [`decode_j2k`], with the time spent in each of its two stages
/// (measurements: `examples/j2k_bench.rs`).
pub fn decode_j2k_timed(data: &[u8], discard: u8) -> Result<(DecodedImage, DecodeTimings), AssetError> {
    let start = Instant::now();
    let cs = locate_codestream(data).ok_or(AssetError::J2k("not a JPEG2000 codestream"))?;
    let info = parse_main_header(cs).ok_or(AssetError::Truncated("JPEG2000 main header"))?;
    if info.width > MAX_DIMENSION || info.height > MAX_DIMENSION || info.width as u64 * info.height as u64 > MAX_PIXELS {
        return Err(AssetError::TooLarge("JPEG2000 image dimensions"));
    }
    if u16::from(info.components) > MAX_HEADER_COMPONENTS {
        return Err(AssetError::Unsupported(format!("JPEG2000 with {} components", info.components)));
    }
    let reduce = discard.min(info.levels);
    let siz = check_decode_memory(cs, reduce)?;
    // (a tile-part can never need more than about twice the raw pixels)
    let plausible = (info.width as usize * info.height as usize * usize::from(info.components)).saturating_mul(2) + (1 << 16);
    let bounded = bound_tile_parts(cs, plausible);
    let cs = bounded.as_deref().unwrap_or(cs);
    // Disable any TLM tile-part index: OpenJPEG 2.5.3 can write past it (see
    // `disable_tlm_index`). Aurora decodes textures in codestream order anyway.
    let no_tlm = disable_tlm_index(cs)?;
    let cs = no_tlm.as_deref().unwrap_or(cs);

    let mut mem = MemStream { data: cs, offset: 0 };

    // SAFETY: plain FFI constructor; null is checked below.
    let stream = StreamGuard(unsafe { opj::opj_stream_create(cs.len().clamp(4096, 1 << 20), 1) });
    if stream.0.is_null() {
        return Err(AssetError::J2k("opj_stream_create failed"));
    }
    // SAFETY: `stream.0` is a valid input stream. The user data pointer refers to
    // `mem`, which is declared before `stream` and therefore dropped after it; no
    // free function is registered because `mem` is owned by this stack frame.
    unsafe {
        opj::opj_stream_set_user_data(stream.0, &mut mem as *mut MemStream<'_> as *mut c_void, None);
        opj::opj_stream_set_user_data_length(stream.0, cs.len() as u64);
        opj::opj_stream_set_read_function(stream.0, Some(mem_read));
        opj::opj_stream_set_skip_function(stream.0, Some(mem_skip));
        opj::opj_stream_set_seek_function(stream.0, Some(mem_seek));
    }

    // SAFETY: plain FFI constructor; null is checked below.
    let codec = CodecGuard(unsafe { opj::opj_create_decompress(opj::CODEC_FORMAT::OPJ_CODEC_J2K) });
    if codec.0.is_null() {
        return Err(AssetError::J2k("opj_create_decompress failed"));
    }

    // SAFETY: `opj_dparameters_t` is a plain C struct (integers and char arrays) for
    // which all-zero is a valid bit pattern; it is then initialised by OpenJPEG.
    let mut params: opj::opj_dparameters_t = unsafe { std::mem::zeroed() };
    // SAFETY: `params` is a valid, exclusively borrowed parameter struct.
    unsafe { opj::opj_set_default_decoder_parameters(&mut params) };
    params.cp_reduce = u32::from(reduce);

    // SAFETY: `codec.0` is a valid decompressor; the callbacks are `extern "C"`
    // functions that ignore their (null) client data.
    let setup_ok = unsafe {
        opj::opj_set_error_handler(codec.0, Some(opj_error_cb), ptr::null_mut());
        opj::opj_set_warning_handler(codec.0, Some(opj_warning_cb), ptr::null_mut());
        opj::opj_set_info_handler(codec.0, Some(opj_info_cb), ptr::null_mut());
        let ok = opj::opj_setup_decoder(codec.0, &mut params);
        // Enable decoding of partially loaded images, as LLImageJ2COJ does.
        opj::opj_decoder_set_strict_mode(codec.0, 0);
        ok
    };
    if setup_ok == 0 {
        return Err(AssetError::J2k("opj_setup_decoder failed"));
    }

    let mut image = ImageGuard(ptr::null_mut());
    // SAFETY: stream and codec are valid; `image.0` receives an OpenJPEG-owned image
    // (or stays null) which the guard destroys.
    let header_ok = unsafe { opj::opj_read_header(stream.0, codec.0, &mut image.0) };
    if header_ok == 0 || image.0.is_null() {
        return Err(AssetError::J2k("opj_read_header failed"));
    }

    // SAFETY: stream, codec and image are valid and belong to this call.
    let decoded = unsafe { opj::opj_decode(codec.0, stream.0, image.0) };
    // SAFETY: as above. The result is ignored like in LL: it fails for truncated
    // streams even though the decoded data is usable.
    unsafe { opj::opj_end_decompress(codec.0, stream.0) };
    if decoded == 0 {
        return Err(AssetError::J2k("opj_decode failed"));
    }

    // SAFETY: `image.0` is non-null and points to an image fully initialised by
    // `opj_read_header`/`opj_decode`; it is not mutated while borrowed.
    let img = unsafe { &*image.0 };
    let codec = start.elapsed();
    let out = convert_image(img, &siz, reduce)?;
    let convert = start.elapsed().saturating_sub(codec);
    Ok((out, DecodeTimings { codec, convert }))
}

/// Width and height of a component of subsampling `sub` decoded at
/// resolution reduction `reduce`, from the SIZ alone, as
/// `opj_j2k_update_image_dimensions` computes them: the image area in the
/// component's grid (`ceil(x / dx)`), then halved `reduce` times, rounding
/// each edge up.
fn component_size(siz: &Siz, sub: (u8, u8), reduce: u8) -> (u64, u64) {
    let span = |a0: u32, a1: u32, d: u8| {
        let d = u64::from(d.max(1));
        let scale = 1u64 << u32::from(reduce).min(63);
        let (c0, c1) = (u64::from(a0).div_ceil(d), u64::from(a1).div_ceil(d));
        c1.div_ceil(scale).saturating_sub(c0.div_ceil(scale))
    };
    (span(siz.x0, siz.x1, sub.0), span(siz.y0, siz.y1, sub.1))
}

/// Convert OpenJPEG component planes to interleaved 8-bit pixels. The
/// components are checked against the codestream's own SIZ (`siz`, decoded
/// at `reduce`) before their planes are read: the same component count, and
/// planes no larger than the SIZ gives them.
fn convert_image(img: &opj::opj_image_t, siz: &Siz, reduce: u8) -> Result<DecodedImage, AssetError> {
    if img.numcomps == 0 || img.comps.is_null() {
        return Err(AssetError::J2k("decoded image has no components"));
    }
    if img.numcomps as usize != siz.sub.len() {
        return Err(AssetError::J2k("decoded component count differs from the header"));
    }
    // SAFETY: OpenJPEG allocates `numcomps` contiguous `opj_image_comp_t` at `comps`.
    let comps = unsafe { std::slice::from_raw_parts(img.comps, img.numcomps as usize) };
    let used = &comps[..comps.len().min(4)];

    let mut width = 0u32;
    let mut height = 0u32;
    for (c, &sub) in used.iter().zip(&siz.sub) {
        if c.data.is_null() || c.w == 0 || c.h == 0 {
            return Err(AssetError::J2k("decoded component has no data"));
        }
        let (max_w, max_h) = component_size(siz, sub, reduce);
        if u64::from(c.w) > max_w || u64::from(c.h) > max_h {
            return Err(AssetError::J2k("decoded component larger than its header"));
        }
        width = width.max(c.w);
        height = height.max(c.h);
    }
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(AssetError::TooLarge("decoded JPEG2000 dimensions"));
    }
    let n = used.len();
    let pixels = width as usize * height as usize;
    let mut out = vec![0u8; pixels * n];

    for (ci, c) in used.iter().enumerate() {
        let cw = c.w as usize;
        let ch = c.h as usize;
        // SAFETY: `data` is non-null and OpenJPEG allocates `w * h` samples for
        // each decoded component (dimensions already reduced by the resolution
        // factor); `w` and `h` were checked above against the SIZ, so the
        // slice never spans more than the header's own component size.
        let plane = unsafe { std::slice::from_raw_parts(c.data as *const i32, cw * ch) };
        let prec = c.prec.clamp(1, 31);
        let offset: i64 = if c.sgnd != 0 { 1i64 << (prec - 1) } else { 0 };
        let max_in = (1i64 << prec) - 1;
        let to8 = |v: i32| -> u8 {
            let v = (v as i64 + offset).clamp(0, max_in);
            let v = match prec.cmp(&8) {
                std::cmp::Ordering::Equal => v,
                std::cmp::Ordering::Greater => v >> (prec - 8),
                std::cmp::Ordering::Less => (v * 255 + max_in / 2) / max_in,
            };
            v.clamp(0, 255) as u8
        };
        let full = cw == width as usize && ch == height as usize;
        for y in 0..height as usize {
            let sy = if full { y } else { (y * ch / height as usize).min(ch - 1) };
            let src_row = &plane[sy * cw..sy * cw + cw];
            let dst_row = &mut out[y * width as usize * n..(y + 1) * width as usize * n];
            if full {
                for (x, &v) in src_row.iter().enumerate() {
                    dst_row[x * n + ci] = to8(v);
                }
            } else {
                for x in 0..width as usize {
                    let sx = (x * cw / width as usize).min(cw - 1);
                    dst_row[x * n + ci] = to8(src_row[sx]);
                }
            }
        }
    }

    Ok(DecodedImage {
        width,
        height,
        components: n as u8,
        data: out,
        discard: comps[0].factor.min(255) as u8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct VecWriter {
        buf: Vec<u8>,
        pos: usize,
    }

    unsafe extern "C" fn w_write(buffer: *mut c_void, n: usize, user: *mut c_void) -> usize {
        // SAFETY: `user` is the VecWriter owned by `encode`, alive for the whole encode.
        let w = unsafe { &mut *(user as *mut VecWriter) };
        // SAFETY: OpenJPEG provides `n` readable bytes at `buffer`.
        let src = unsafe { std::slice::from_raw_parts(buffer as *const u8, n) };
        let end = w.pos + n;
        if w.buf.len() < end {
            w.buf.resize(end, 0);
        }
        w.buf[w.pos..end].copy_from_slice(src);
        w.pos = end;
        n
    }

    unsafe extern "C" fn w_skip(n: i64, user: *mut c_void) -> i64 {
        // SAFETY: see `w_write`.
        let w = unsafe { &mut *(user as *mut VecWriter) };
        let new = w.pos as i64 + n;
        if new < 0 {
            return -1;
        }
        w.pos = new as usize;
        if w.buf.len() < w.pos {
            w.buf.resize(w.pos, 0);
        }
        n
    }

    unsafe extern "C" fn w_seek(off: i64, user: *mut c_void) -> opj::OPJ_BOOL {
        // SAFETY: see `w_write`.
        let w = unsafe { &mut *(user as *mut VecWriter) };
        if off < 0 {
            return 0;
        }
        w.pos = off as usize;
        if w.buf.len() < w.pos {
            w.buf.resize(w.pos, 0);
        }
        1
    }

    /// Lossless J2K encode (test helper, mirrors JPEG2KEncode in llimagej2coj.cpp).
    fn encode(w: u32, h: u32, comps: u32, pixel: impl Fn(u32, u32, u32) -> u8) -> Vec<u8> {
        // SAFETY: test-only FFI usage; every pointer is checked and freed below.
        unsafe {
            let mut params: opj::opj_cparameters_t = std::mem::zeroed();
            opj::opj_set_default_encoder_parameters(&mut params);
            params.tcp_numlayers = 1;
            params.tcp_rates[0] = 0.0;
            params.cp_disto_alloc = 1;
            params.irreversible = 0;
            params.tcp_mct = if comps >= 3 { 1 } else { 0 };
            // Like JPEG2KEncode: small images need fewer resolutions.
            params.numresolution = params.numresolution.min(1 + w.min(h).ilog2() as i32);
            let mut cparms: Vec<opj::opj_image_cmptparm_t> = (0..comps)
                .map(|_| {
                    let mut p: opj::opj_image_cmptparm_t = std::mem::zeroed();
                    p.dx = 1;
                    p.dy = 1;
                    p.w = w;
                    p.h = h;
                    p.prec = 8;
                    p.sgnd = 0;
                    p
                })
                .collect();
            let image = opj::opj_image_create(comps, cparms.as_mut_ptr(), opj::COLOR_SPACE::OPJ_CLRSPC_SRGB);
            assert!(!image.is_null());
            (*image).x0 = 0;
            (*image).y0 = 0;
            (*image).x1 = w;
            (*image).y1 = h;
            for c in 0..comps {
                let comp = &*(*image).comps.add(c as usize);
                let plane = std::slice::from_raw_parts_mut(comp.data, (w * h) as usize);
                for y in 0..h {
                    for x in 0..w {
                        plane[(y * w + x) as usize] = pixel(x, y, c) as i32;
                    }
                }
            }
            let codec = opj::opj_create_compress(opj::CODEC_FORMAT::OPJ_CODEC_J2K);
            assert!(!codec.is_null());
            assert!(opj::opj_setup_encoder(codec, &mut params, image) != 0);
            let mut writer = VecWriter { buf: Vec::new(), pos: 0 };
            let stream = opj::opj_stream_create(1 << 16, 0);
            opj::opj_stream_set_user_data(stream, &mut writer as *mut VecWriter as *mut c_void, None);
            opj::opj_stream_set_write_function(stream, Some(w_write));
            opj::opj_stream_set_skip_function(stream, Some(w_skip));
            opj::opj_stream_set_seek_function(stream, Some(w_seek));
            assert!(opj::opj_start_compress(codec, image, stream) != 0);
            assert!(opj::opj_encode(codec, stream) != 0);
            assert!(opj::opj_end_compress(codec, stream) != 0);
            opj::opj_stream_destroy(stream);
            opj::opj_destroy_codec(codec);
            opj::opj_image_destroy(image);
            writer.buf
        }
    }

    fn pattern(x: u32, y: u32, c: u32) -> u8 {
        ((x * 4 + y * 3 + c * 50) & 0xFF) as u8
    }

    #[test]
    fn info_and_decode_rgb() {
        let enc = encode(64, 48, 3, pattern);
        let info = j2k_info(&enc).expect("info");
        assert_eq!(
            info,
            J2kInfo {
                width: 64,
                height: 48,
                components: 3,
                levels: 5
            }
        );

        let img = decode_j2k(&enc, 0).expect("decode d0");
        assert_eq!((img.width, img.height, img.components, img.discard), (64, 48, 3, 0));
        assert_eq!(img.data.len(), 64 * 48 * 3);
        for y in 0..48u32 {
            for x in 0..64u32 {
                for c in 0..3u32 {
                    assert_eq!(img.data[((y * 64 + x) * 3 + c) as usize], pattern(x, y, c));
                }
            }
        }

        let img1 = decode_j2k(&enc, 1).expect("decode d1");
        assert_eq!((img1.width, img1.height, img1.components, img1.discard), (32, 24, 3, 1));
        assert_eq!(img1.data.len(), 32 * 24 * 3);

        // Discard beyond the number of levels is clamped.
        let img9 = decode_j2k(&enc, 9).expect("decode d9");
        assert_eq!(img9.discard, 5);
        assert_eq!((img9.width, img9.height), (2, 2));
    }

    #[test]
    fn decode_gray_and_rgba() {
        let enc = encode(16, 16, 1, |x, y, _| (x * 16 + y) as u8);
        let img = decode_j2k(&enc, 0).expect("gray");
        assert_eq!(img.components, 1);
        assert_eq!(img.data[5 * 16 + 3], (3 * 16 + 5) as u8);

        let enc = encode(32, 32, 4, pattern);
        let img = decode_j2k(&enc, 0).expect("rgba");
        assert_eq!(img.components, 4);
        assert_eq!(img.data[(7 * 32 + 9) * 4 + 3], pattern(9, 7, 3));
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(decode_j2k(&[], 0).is_err());
        assert!(decode_j2k(b"definitely not a jpeg2000 file", 0).is_err());
        let mut junk = vec![0xFF, 0x4F, 0xFF, 0x51];
        junk.extend((0..500u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8));
        assert!(decode_j2k(&junk, 0).is_err());
        assert!(j2k_info(&junk).is_none());
    }

    #[test]
    fn truncated_does_not_panic() {
        let enc = encode(128, 128, 3, pattern);
        for cut in [10, 60, 120, 200, enc.len() / 4, enc.len() / 2, enc.len() - 3] {
            let r = decode_j2k(&enc[..cut], 0);
            if let Ok(img) = r {
                assert_eq!(img.data.len(), (img.width * img.height * img.components as u32) as usize);
            }
        }
        // A cut that keeps the header must still decode (non-strict mode).
        let img = decode_j2k(&enc[..enc.len() / 2], 0).expect("partial decode");
        assert_eq!((img.width, img.height), (128, 128));
    }

    #[test]
    fn concurrent_decodes() {
        use rayon::prelude::*;
        let enc = encode(64, 64, 3, pattern);
        let ok = (0..32).into_par_iter().map(|i| decode_j2k(&enc, (i % 3) as u8).is_ok()).all(|b| b);
        assert!(ok);
    }

    #[test]
    fn data_size_estimate() {
        let info = J2kInfo {
            width: 1024,
            height: 1024,
            components: 4,
            levels: 5,
        };
        // Non-increasing with discard, always including the 600 byte header.
        let sizes: Vec<usize> = (0..=5).map(|d| bytes_for_discard(&info, d)).collect();
        for w in sizes.windows(2) {
            assert!(w[0] >= w[1]);
        }
        assert!(sizes[0] > sizes[2]);
        // discard 5 => only the base 64x64 layer: 64*64*32/8 bits / 8 + 600
        assert_eq!(sizes[5], 64 * 64 * 32 / 8 / 8 + 600);
        // Clamped like LLImageJ2C::calcDataSize.
        assert_eq!(bytes_for_discard(&info, 200), sizes[5]);
    }

    /// Found by fuzzing (libFuzzer): an 8x8 codestream whose tile-part claims
    /// 0x400000BF bytes made OpenJPEG allocate 1 GB before reading them.
    #[test]
    fn a_tile_part_longer_than_the_codestream_is_bounded() {
        let cs: &[u8] = &[
            0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x2F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03,
            0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0xFF, 0x52, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x02, 0x01, 0x03, 0x04, 0x04,
            0x00, 0x00, 0xFF, 0x5C, 0x00, 0x17, 0x42, 0x67, 0x38, 0x67, 0x50, 0x67, 0x50, 0x67, 0x68, 0x50, 0x05, 0x50, 0x05, 0x50, 0x47,
            0x57, 0xD3, 0x57, 0xD3, 0x57, 0x62, 0xFF, 0x64, 0x00, 0x25, 0x00, 0x01, 0x43, 0x72, 0x65, 0x61, 0x74, 0x65, 0x64, 0x20, 0x62,
            0x79, 0x20, 0x4F, 0x70, 0x65, 0x6E, 0x4A, 0x50, 0x45, 0x47, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6F, 0x6E, 0x20, 0x32, 0x2E,
            0x35, 0x2E, 0x33, 0xFF, 0x90, 0x00, 0x0A, 0x00, 0x00, 0x40, 0x00, 0x00, 0xBF, 0x00, 0x01, 0xFF, 0x93, 0x80, 0x80, 0x80, 0x80,
            0x80, 0x80, 0x80, 0x80, 0x80, 0x91, 0xD1, 0x80, 0x0B, 0x95, 0xE2, 0x01, 0xE8, 0xFF, 0xD9,
        ];
        let sot = 0x81;
        assert_eq!(be32(cs, sot + 6), Some(0x4000_00BF));
        let bounded = bound_tile_parts(cs, 8 * 8 * 3 * 2 + (1 << 16)).expect("patched copy");
        // a partial download keeps its tile-part lengths
        assert!(bound_tile_parts(cs, usize::MAX).is_none());
        assert_eq!(bounded.len(), cs.len());
        assert_eq!(be32(&bounded, sot + 6), Some(0));
        assert_eq!(&bounded[..sot + 6], &cs[..sot + 6]);
        assert_eq!(&bounded[sot + 10..], &cs[sot + 10..]);
        // a codestream our encoder wrote is left as it is
        let pixels: Vec<u8> = (0..64u32 * 64 * 3).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
        let good = encode_texture_j2k(64, 64, 3, &pixels, false).expect("encode");
        let good = locate_codestream(&good).expect("codestream");
        assert!(bound_tile_parts(good, 0).is_none());
        assert!(
            bound_tile_parts(&good[..good.len() / 2], 0).is_some(),
            "cut short, an implausible length"
        );
        let _ = decode_j2k(cs, 0);
    }
    /// Found by the second Fuzz workflow run: the main header ends with a run of
    /// 0xFF fill bytes that OpenJPEG skips to find the SOT behind them, which
    /// claims 0x6000003C bytes (1.6 GB).
    #[test]
    fn a_tile_part_behind_fill_bytes_is_bounded() {
        let cs: &[u8] = &[
            0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x32, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04,
            0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0x07, 0x05, 0x01, 0xFF, 0x52, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x02, 0x01,
            0x03, 0x04, 0x04, 0x00, 0x00, 0xFF, 0x5C, 0x00, 0x17, 0x42, 0x67, 0x38, 0x67, 0x50, 0x67, 0x50, 0x67, 0x68, 0x50, 0x05, 0x50,
            0x05, 0x50, 0x47, 0x57, 0xD3, 0x57, 0xD3, 0x57, 0x62, 0xFF, 0x64, 0x00, 0x25, 0x00, 0x01, 0x43, 0x72, 0x65, 0x61, 0x74, 0x65,
            0x64, 0x20, 0x62, 0x79, 0x20, 0x4F, 0x70, 0x65, 0x7E, 0x4A, 0x50, 0x45, 0x5D, 0x47, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6F,
            0x6E, 0x20, 0x33, 0x2E, 0x35, 0x2E, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x33, 0xFF, 0x90, 0x00, 0x0A, 0x00, 0x00, 0x60, 0x00, 0x00, 0x3C, 0x00, 0x01, 0xFF, 0x93, 0x80,
            0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x91, 0xC6, 0x00, 0x11, 0x85, 0xFE, 0x0E, 0xC6, 0xDA, 0x97,
            0x23, 0x9E, 0xA6, 0xF9, 0x99, 0x80, 0x80, 0x80, 0x78, 0x7F, 0x80, 0x80, 0x80, 0x80, 0x80, 0x57, 0x57, 0x57, 0x57, 0x57, 0x80,
            0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0xFF, 0x80, 0xFF, 0xD9,
        ];
        let sot = 0x102;
        assert_eq!(be32(cs, sot + 6), Some(0x6000_003C));
        let bounded = bound_tile_parts(cs, 16 * 8 * 4 * 2 + (1 << 16)).expect("patched copy");
        assert_eq!(be32(&bounded, sot + 6), Some(0));
        let _ = decode_j2k(cs, 0);
    }
    /// The header fields a synthetic codestream is built from.
    #[derive(Clone)]
    struct Layout {
        /// Image area on the reference grid: x0, y0, x1, y1.
        area: (u32, u32, u32, u32),
        /// Tile grid origin and tile size.
        tile_origin: (u32, u32),
        tile: (u32, u32),
        comps: u16,
        sub: (u8, u8),
        levels: u8,
        layers: u16,
        /// xcb, ycb as written (log2 of the code-block size minus 2).
        block: (u8, u8),
        /// Precinct size byte (PPy << 4 | PPx) for every resolution.
        precinct: Option<u8>,
    }

    impl Layout {
        fn texture(w: u32, h: u32, comps: u16) -> Self {
            Layout {
                area: (0, 0, w, h),
                tile_origin: (0, 0),
                tile: (w, h),
                comps,
                sub: (1, 1),
                levels: 5,
                layers: 1,
                block: (4, 4),
                precinct: None,
            }
        }
    }

    fn segment(marker: u16, body: &[u8]) -> Vec<u8> {
        let mut s = marker.to_be_bytes().to_vec();
        s.extend_from_slice(&(body.len() as u16 + 2).to_be_bytes());
        s.extend_from_slice(body);
        s
    }

    /// COD contents: Scod, progression, layers, mct, then SPcod.
    fn cod_body(l: &Layout) -> Vec<u8> {
        let mut b = vec![u8::from(l.precinct.is_some()), 0];
        b.extend_from_slice(&l.layers.to_be_bytes());
        b.extend_from_slice(&[0, l.levels, l.block.0, l.block.1, 0, 1]);
        if let Some(p) = l.precinct {
            b.extend(std::iter::repeat_n(p, usize::from(l.levels) + 1));
        }
        b
    }

    /// SOC, SIZ, COD, QCD, then one tile-part whose header holds
    /// `tile_header` and whose data is empty, and EOC.
    fn codestream(l: &Layout, tile_header: &[u8]) -> Vec<u8> {
        let mut siz = vec![0, 0];
        for v in [
            l.area.2,
            l.area.3,
            l.area.0,
            l.area.1,
            l.tile.0,
            l.tile.1,
            l.tile_origin.0,
            l.tile_origin.1,
        ] {
            siz.extend_from_slice(&v.to_be_bytes());
        }
        siz.extend_from_slice(&l.comps.to_be_bytes());
        for _ in 0..l.comps {
            siz.extend_from_slice(&[7, l.sub.0, l.sub.1]);
        }
        let mut cs = vec![0xFF, 0x4F];
        cs.extend(segment(0xFF51, &siz));
        cs.extend(segment(0xFF52, &cod_body(l)));
        // No quantization, 2 guard bits: one exponent per band.
        let mut qcd = vec![0x40];
        qcd.extend(std::iter::repeat_n(8 << 3, 3 * usize::from(l.levels) + 1));
        cs.extend(segment(0xFF5C, &qcd));
        cs.extend_from_slice(&[0xFF, 0x90, 0x00, 0x0A, 0, 0, 0, 0, 0, 0, 0, 1]);
        cs.extend_from_slice(tile_header);
        cs.extend_from_slice(&[0xFF, 0x93, 0xFF, 0xD9]);
        cs
    }

    fn memory(cs: &[u8], reduce: u8) -> u64 {
        let (siz, coding) = parse_layout(cs).expect("layout");
        decode_memory(&siz, &coding, reduce, tile_grid(&siz).expect("tile grid"))
    }

    /// Refused before OpenJPEG sees it, at every discard level.
    fn assert_refused(cs: &[u8]) {
        for discard in 0..=MAX_DISCARD_LEVEL {
            assert!(
                matches!(check_decode_memory(cs, discard), Err(AssetError::TooLarge(_))),
                "discard {discard}"
            );
            assert!(matches!(decode_j2k(cs, discard), Err(AssetError::TooLarge(_))));
        }
    }

    #[test]
    fn the_synthetic_layout_is_read_back() {
        let mut l = Layout::texture(640, 480, 3);
        l.tile = (256, 128);
        l.layers = 7;
        l.block = (3, 2);
        l.precinct = Some(0x87);
        let cs = codestream(&l, &[]);
        let (siz, coding) = parse_layout(&cs).expect("layout");
        assert_eq!((siz.x1, siz.y1, siz.tile_w, siz.tile_h, siz.sub.len()), (640, 480, 256, 128, 3));
        assert_eq!(tile_grid(&siz), Some((3, 4)));
        assert_eq!(
            coding,
            Coding {
                levels: 5,
                layers: 7,
                block: (5, 4),
                precinct: (7, 8)
            }
        );
        assert!(check_decode_memory(&cs, 0).is_ok());
        let _ = decode_j2k(&cs, 0);
    }

    /// What the textures of the grid look like stays well within the budget.
    #[test]
    fn real_texture_layouts_are_accepted() {
        for (w, comps) in [(64, 3), (512, 4), (1024, 4), (2048, 4), (4096, 4), (8192, 1)] {
            let l = Layout::texture(w, w, comps);
            let cs = codestream(&l, &[]);
            assert!(check_decode_memory(&cs, 0).is_ok(), "{w}² × {comps}");
        }
        // An SL texture of 2048² RGBA at full resolution: about 85 MB.
        let big = memory(&codestream(&Layout::texture(2048, 2048, 4), &[]), 0);
        assert!(big < 100 << 20, "{big}");
        // Tiled and with precincts, as other encoders write them.
        let mut l = Layout::texture(2048, 2048, 4);
        l.tile = (256, 256);
        l.layers = 12;
        l.precinct = Some(0x77);
        assert!(check_decode_memory(&codestream(&l, &[]), 0).is_ok());
        l.tile = (64, 64);
        l.precinct = None;
        assert!(check_decode_memory(&codestream(&l, &[]), 0).is_ok());
        // Every texture our encoder writes.
        let pixels: Vec<u8> = (0..256u32 * 256 * 4).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
        for comps in [3u8, 4] {
            let n = 256 * 256 * usize::from(comps);
            let enc = encode_texture_j2k(256, 256, comps, &pixels[..n], false).expect("encode");
            assert!(check_decode_memory(locate_codestream(&enc).expect("codestream"), 0).is_ok());
        }
    }

    /// Full-size samples of many components, at every discard level.
    #[test]
    fn many_large_components_are_refused() {
        let mut l = Layout::texture(8192, 8192, 16);
        l.levels = 0;
        assert_refused(&codestream(&l, &[]));
        // An RGBA 8192² at full resolution needs about 1.3 GB; a level down it fits.
        let cs = codestream(&Layout::texture(8192, 8192, 4), &[]);
        assert!(check_decode_memory(&cs, 0).is_err());
        assert!(check_decode_memory(&cs, 1).is_ok());
    }

    /// Tiny tiles: OpenJPEG keeps the coding parameters of each one.
    #[test]
    fn tile_counts_are_bounded() {
        let mut l = Layout::texture(1024, 1024, 4);
        l.tile = (8, 8);
        assert_refused(&codestream(&l, &[]));
        // Counted from the tile grid origin, which lies in the first tile.
        let mut l = Layout::texture(64, 64, 4);
        l.area = (1 << 20, 0, (1 << 20) + 64, 64);
        l.tile_origin = ((1 << 20) - 1, 0);
        let cs = codestream(&l, &[]);
        let (siz, _) = parse_layout(&cs).expect("layout");
        assert_eq!(tile_grid(&siz), Some((2, 1)));
        assert!(check_decode_memory(&cs, 0).is_ok());
        // A tile grid OpenJPEG refuses itself is refused here too.
        for origin in [0, (1 << 20) + 1] {
            l.tile_origin = (origin, 0);
            assert!(check_decode_memory(&codestream(&l, &[]), 0).is_err());
        }
    }

    /// Code-blocks of 4 × 4 samples in 2 × 2 precincts: a structure every few
    /// samples, at every resolution whatever the discard level.
    #[test]
    fn small_code_blocks_and_precincts_are_refused() {
        let mut l = Layout::texture(4096, 4096, 4);
        l.block = (0, 0);
        l.precinct = Some(0x11);
        assert_refused(&codestream(&l, &[]));
    }

    /// The packet iterator flags one OPJ_INT16 per layer and precinct.
    #[test]
    fn many_layers_are_refused() {
        let mut l = Layout::texture(2048, 2048, 4);
        l.precinct = Some(0x44);
        assert!(check_decode_memory(&codestream(&l, &[]), 0).is_ok());
        l.layers = u16::MAX;
        assert_refused(&codestream(&l, &[]));
    }

    /// A COD in a tile-part header replaces the main header's for its tile.
    #[test]
    fn tile_part_coding_styles_are_counted() {
        let l = Layout::texture(4096, 4096, 4);
        assert!(check_decode_memory(&codestream(&l, &[]), 0).is_ok());
        let mut costly = l.clone();
        costly.block = (0, 0);
        costly.precinct = Some(0x11);
        let cod = segment(0xFF52, &cod_body(&costly));
        assert_refused(&codestream(&l, &cod));
        // also behind fill bytes, which OpenJPEG passes to find the next marker
        let mut hidden = vec![0xFF; 6];
        hidden.extend(&cod);
        assert_refused(&codestream(&l, &hidden));
        // and as a COC for one component
        let mut coc = vec![2, 1];
        coc.extend_from_slice(&cod_body(&costly)[5..]);
        assert_refused(&codestream(&l, &segment(0xFF53, &coc)));
    }

    /// A codestream whose main header holds a TLM segment listing one
    /// tile-part of 14 bytes for tile 0, followed by `parts` empty tile-parts
    /// of that tile in order (TPsot 0, 1, …), none giving the tile-part count
    /// (TNsot 0).
    fn codestream_with_tlm(parts: u8) -> Vec<u8> {
        codestream_with_tlm_after(&[], parts)
    }

    /// [`codestream_with_tlm`], with `before` placed in the main header just
    /// before the TLM segment.
    fn codestream_with_tlm_after(before: &[u8], parts: u8) -> Vec<u8> {
        let main = codestream(&Layout::texture(64, 64, 3), &[]);
        let sot = main.windows(4).position(|w| w == [0xFF, 0x90, 0x00, 0x0A]).expect("SOT");
        let mut cs = main[..sot].to_vec();
        cs.extend_from_slice(before);
        // Ztlm 0, Stlm: 8-bit Ttlm, 16-bit Ptlm; Ttlm 0, Ptlm 14.
        cs.extend(segment(0xFF55, &[0, 0x10, 0, 0, 14]));
        for part in 0..parts {
            cs.extend_from_slice(&[0xFF, 0x90, 0x00, 0x0A, 0, 0, 0, 0, 0, 14, part, 0, 0xFF, 0x93]);
        }
        cs.extend_from_slice(&[0xFF, 0xD9]);
        cs
    }

    /// OpenJPEG sizes its tile-part index from the TLM segments, then fills
    /// it from the SOT markers: tile-parts the TLM does not list, with no
    /// TNsot to resize it, wrote past its end (heap overflow). The main
    /// header's TLM is disabled before OpenJPEG reads it, so the index grows
    /// from the SOT markers instead.
    #[test]
    fn tile_parts_beyond_the_tlm_are_read_in_stream_order() {
        let cs = codestream_with_tlm(200);
        // The TLM is found and disabled (its Stlm byte forced to ST = 3).
        assert_tlm_disabled(&cs);
        // A codestream with no TLM is left untouched.
        let plain = codestream(&Layout::texture(64, 64, 3), &[]);
        assert!(disable_tlm_index(&plain).expect("header walked").is_none());
        // Decoding the crafted stream must not corrupt memory.
        let _ = decode_j2k(&cs, 0);
    }

    /// The last TLM of `cs` has its Stlm byte forced to ST = 3, and nothing
    /// else changes.
    fn assert_tlm_disabled(cs: &[u8]) {
        let patched = disable_tlm_index(cs).expect("header walked").expect("TLM disabled");
        // FF 55, Lcme(2), Ztlm(1), Stlm(1): Stlm is the sixth byte.
        let tlm = cs.windows(2).rposition(|w| w == [0xFF, 0x55]).expect("TLM marker");
        assert_eq!(patched[tlm + 5] & 0x30, 0x30, "Stlm ST bits set");
        assert_eq!(&patched[..tlm + 5], &cs[..tlm + 5], "only the Stlm byte changes");
        assert_eq!(&patched[tlm + 6..], &cs[tlm + 6..]);
    }

    /// OpenJPEG passes the bytes after an unknown marker two at a time until
    /// it finds a known one (`opj_j2k_read_unk`), SOD and EOC included, which
    /// it does not know in a header. Whatever comes before the TLM, the walk
    /// that disables it must reach it the same way.
    #[test]
    fn a_tlm_behind_unknown_markers_is_still_disabled() {
        let hiders: [&[u8]; 7] = [
            &[0xFF, 0x31, 0x00, 0x00], // unknown marker, length 0
            &[0xFF, 0x31, 0x00, 0x01], // unknown marker, length 1
            &[0xFF, 0x31, 0xFF, 0xFF], // unknown marker, huge length
            &[0xFF, 0x6F],             // unknown marker, nothing after it
            &[0xFF, 0x4F, 0x00, 0x00], // a second SOC
            &[0xFF, 0x93],             // SOD
            &[0xFF, 0xD9],             // EOC
        ];
        for before in hiders {
            let cs = codestream_with_tlm_after(before, 200);
            assert_tlm_disabled(&cs);
            let _ = decode_j2k(&cs, 0);
        }
    }

    /// A main header the walk cannot follow to its first SOT is refused
    /// rather than handed to OpenJPEG with its TLM, if any, still on.
    #[test]
    fn a_main_header_that_does_not_reach_a_sot_is_refused() {
        let cs = codestream_with_tlm(4);
        let tlm = cs.windows(2).position(|w| w == [0xFF, 0x55]).expect("TLM marker");
        // A known segment too short to hold its length.
        let mut short = cs[..tlm].to_vec();
        short.extend_from_slice(&[0xFF, 0x64, 0x00, 0x01]);
        short.extend_from_slice(&cs[tlm..]);
        assert!(disable_tlm_index(&short).is_err());
        assert!(decode_j2k(&short, 0).is_err());
        // A TLM too short to hold its Stlm byte.
        let mut no_stlm = cs[..tlm].to_vec();
        no_stlm.extend(segment(0xFF55, &[0]));
        no_stlm.extend_from_slice(&cs[tlm..]);
        assert!(disable_tlm_index(&no_stlm).is_err());
        // A main header cut before its SOT.
        assert!(disable_tlm_index(&cs[..tlm + 4]).is_err());
    }

    /// Found by fuzzing: a SIZ header claiming 2^32 - 1 on both sides
    /// overflowed the surface (a panic in the overflow-checked CI profile).
    #[test]
    fn data_size_estimate_of_the_largest_claimed_size() {
        let huge = J2kInfo {
            width: u32::MAX,
            height: u32::MAX,
            components: 4,
            levels: 5,
        };
        assert!(bytes_for_discard(&huge, 0) > bytes_for_discard(&huge, 5));
    }

    /// The component sizes `convert_image` allows follow
    /// `opj_j2k_update_image_dimensions`: the area in the component's grid,
    /// then each reduction rounding both edges up.
    #[test]
    fn component_sizes_follow_the_siz() {
        let mut l = Layout::texture(64, 48, 3);
        let siz = parse_layout(&codestream(&l, &[])).expect("layout").0;
        assert_eq!(component_size(&siz, (1, 1), 0), (64, 48));
        assert_eq!(component_size(&siz, (1, 1), 1), (32, 24));
        assert_eq!(component_size(&siz, (1, 1), 5), (2, 2));
        l.area = (1, 1, 9, 9);
        l.tile_origin = (1, 1);
        l.tile = (8, 8);
        let siz = parse_layout(&codestream(&l, &[])).expect("layout").0;
        // x: ceil(1/2) = 1 to ceil(9/2) = 5 → 4; halved: ceil(5/2) - ceil(1/2) = 2.
        assert_eq!(component_size(&siz, (2, 2), 0), (4, 4));
        assert_eq!(component_size(&siz, (2, 2), 1), (2, 2));
        assert_eq!(component_size(&siz, (1, 1), 0), (8, 8));
        // A zero subsampling (which OpenJPEG refuses) does not divide by zero.
        assert_eq!(component_size(&siz, (0, 0), 0), (8, 8));
        // Every real decode stays within those sizes.
        let enc = encode(64, 48, 3, pattern);
        for discard in 0..=5 {
            let img = decode_j2k(&enc, discard).expect("decode");
            let siz = parse_layout(locate_codestream(&enc).expect("codestream")).expect("layout").0;
            let (w, h) = component_size(&siz, (1, 1), discard);
            assert_eq!((u64::from(img.width), u64::from(img.height)), (w, h));
        }
    }

    /// The main header is walked as OpenJPEG reads it: an unknown marker is
    /// passed two bytes at a time, never by the length it declares, so the
    /// image info and the decoder's budget read the same segments as OpenJPEG.
    #[test]
    fn the_main_header_is_read_like_openjpeg() {
        let l = Layout::texture(64, 64, 3);
        let cs = codestream(&l, &[]);
        let cod = cs.windows(2).position(|w| w == [0xFF, 0x52]).expect("COD");
        // An unknown marker (0xFF6F) whose length covers the whole COD.
        let cod_len = usize::from(be16(&cs, cod + 2).expect("Lcod")) + 2;
        let mut hidden = cs[..cod].to_vec();
        hidden.extend_from_slice(&[0xFF, 0x6F]);
        hidden.extend_from_slice(&(cod_len as u16 + 2).to_be_bytes());
        hidden.extend_from_slice(&cs[cod..]);
        let info = parse_main_header(&hidden).expect("COD found behind the unknown marker");
        assert_eq!(info.levels, 5);
        assert_eq!(parse_layout(&hidden).expect("layout").1.levels, 5);
        // Only the first SIZ counts.
        let siz = cs.windows(2).position(|w| w == [0xFF, 0x51]).expect("SIZ");
        let siz_len = usize::from(be16(&cs, siz + 2).expect("Lsiz")) + 2;
        let other = codestream(&Layout::texture(32, 32, 3), &[]);
        let mut twice = cs[..cod].to_vec();
        twice.extend_from_slice(&other[siz..siz + siz_len]);
        twice.extend_from_slice(&cs[cod..]);
        assert_eq!((parse_main_header(&twice).expect("info").width), 64);
        assert_eq!(parse_layout(&twice).expect("layout").0.x1, 64);
    }
}
