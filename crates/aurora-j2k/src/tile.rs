// This file is a port and keeps the notices of what it ports (both under the
// BSD 2-Clause License, whose text NOTICE.md gives, "JPEG 2000 decoder").
//
// neo-j2k, `src/tile.cpp`:
//   Copyright (c) 2026, Anne Skydancer (neo-j2k)
// OpenJPEG, through it:
//   Copyright (c) 2002-2014, Universite catholique de Louvain (UCL), Belgium
//   Copyright (c) 2002-2014, Professor Benoit Macq
//   Copyright (c) 2001-2003, David Janssens
//   Copyright (c) 2002-2003, Yannick Verschueren
//   Copyright (c) 2003-2007, Francois-Olivier Devaux
//   Copyright (c) 2003-2014, Antonin Descampe
//   Copyright (c) 2005, Herve Drolon, FreeImage Team
//   Copyright (c) 2006-2007, Parvatha Elangovan
//   Copyright (c) 2008, 2011-2012, Centre National d'Etudes Spatiales (CNES), FR
//   Copyright (c) 2012, CS Systemes d'Information, France
//   Copyright (c) 2017, IntoPIX SA <support@intopix.com>
//   All rights reserved.

//! One tile's decode: its layout (tile-components, resolutions, bands,
//! precincts, code-blocks) and its samples (Tier-1, the inverse wavelet and
//! the colour transform's checks). Its packets are Tier-2's (`t2.rs`).
//!
//! Port of neo-j2k's `src/tile.cpp` and `src/tile.h` (BSD-2-Clause,
//! Copyright (c) 2026 Anne Skydancer), after OpenJPEG 2.5.4's `tcd.c`
//! (`opj_tcd_init_tile`, `opj_tcd_decode_tile`; BSD-2-Clause, see NOTICE.md).
//! The layout is flat arrays made from the header model with OpenJPEG's
//! geometry, so every packet and every code-block is where OpenJPEG has it.
//! The arrays are kept between decodes: a decoder that has decoded an image
//! of some size decodes the next one of that size without a heap call.

use crate::Error;
use crate::dwt::{self, DwtScratch, ResRect, RowLayout};
use crate::header::{CBLK_HT, MAX_RESOLUTIONS, MainHeader, TileComponentParams, ceildivpow2};
use crate::mct::{Finish, Mct};
use crate::t1::{self, T1, T1Block};

/// The bytes of one contribution (a codeword segment's part from one
/// packet), as a range of the tile's data.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Chunk {
    pub pos: u32,
    pub len: u32,
}

/// A codeword segment of a code-block (`opj_tcd_seg_t`, decoding side).
/// Tier-1 reads `len` (the bytes with data) and `real_num_passes` (the
/// passes with data); the others are Tier-2's state.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Segment {
    pub len: u32,
    pub numpasses: u32,
    pub real_num_passes: u32,
    pub maxpasses: u32,
    pub numnewpasses: u32,
    pub newlen: u32,
}

/// A code-block's decoding state (`opj_tcd_cblk_dec_t`).
#[derive(Debug, Default)]
pub(crate) struct CodeBlock {
    pub segs: Vec<Segment>,
    pub chunks: Vec<Chunk>,
    pub numbps: u32,
    pub numlenbits: u32,
    pub numsegs: u32,
    pub real_num_segs: u32,
    pub numnewpasses: u32,
    pub corrupted: bool,
}

impl CodeBlock {
    /// Back to a code-block no packet has touched, its lists' memory kept.
    fn reset(&mut self) {
        self.segs.clear();
        self.chunks.clear();
        self.numbps = 0;
        self.numlenbits = 0;
        self.numsegs = 0;
        self.real_num_segs = 0;
        self.numnewpasses = 0;
        self.corrupted = false;
    }
}

/// A tag tree's node (`opj_tgt_node_t` without the encoder's `known`).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TagNode {
    pub value: i32,
    pub low: i32,
}

/// A tag tree: its nodes in `Tile::tag_nodes` from `off` on, leaves first,
/// then each level up to the root, a node's parent the one at `(x / 2,
/// y / 2)` of the next level. No tree when a side is 0.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TagTree {
    pub off: u32,
    pub w: u32,
    pub h: u32,
}

impl TagTree {
    /// Nodes of every level (`opj_tgt_create`).
    pub fn node_count(w: u32, h: u32) -> u64 {
        if u64::from(w) * u64::from(h) == 0 {
            return 0;
        }
        let (mut lw, mut lh) = (u64::from(w), u64::from(h));
        let mut total = 0u64;
        loop {
            let n = lw * lh;
            total += n;
            if n <= 1 {
                return total;
            }
            lw = lw.div_ceil(2);
            lh = lh.div_ceil(2);
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Precinct {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    /// Code-blocks across and down.
    pub cw: u32,
    pub ch: u32,
    /// The code-block grid's origin (`tlcblkxstart`, `tlcblkystart`).
    pub cblk_x0: i32,
    pub cblk_y0: i32,
    /// First code-block in `Tile::cblks`; `cw * ch` of them.
    pub cblks: u32,
    pub incl: TagTree,
    pub imsb: TagTree,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Band {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    /// 0 for resolution 0, else 1 HL, 2 LH, 3 HH.
    pub bandno: u32,
    pub numbps: i32,
    pub stepsize: f32,
    /// First precinct in `Tile::precincts`; `pw * ph` of the resolution.
    pub precincts: u32,
}

impl Band {
    pub fn empty(&self) -> bool {
        self.x1 - self.x0 == 0 || self.y1 - self.y0 == 0
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Resolution {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub pw: u32,
    pub ph: u32,
    pub numbands: u32,
    /// Precinct exponents.
    pub pdx: u32,
    pub pdy: u32,
    pub cblkw_expn: u32,
    pub cblkh_expn: u32,
    pub bands: [Band; 3],
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TileComp {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub numresolutions: u32,
    pub minimum_num_resolutions: u32,
    /// First resolution in `Tile::res`.
    pub res: u32,
    pub dc_shift: i32,
    pub cblksty: u8,
    /// The 5/3 wavelet (`qmfbid == 1`), else the 9/7.
    pub reversible: bool,
    pub roishift: u8,
    /// Whether the component's samples are decoded (Tier-1, wavelet); its
    /// packets are read either way.
    pub decoded: bool,
    /// The samples of the decoded resolution (`minimum_num_resolutions - 1`),
    /// `data_w` x `data_h`, int32 or float bits, in `Scratch::planes`.
    pub data_w: u32,
    pub data_h: u32,
}

/// A tile being decoded. The lists are the layout's flat arrays; the tile's
/// bytes and its samples live with the caller.
#[derive(Debug, Default)]
pub(crate) struct Tile {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub comps: Vec<TileComp>,
    pub res: Vec<Resolution>,
    pub precincts: Vec<Precinct>,
    pub cblks: Vec<CodeBlock>,
    pub tag_nodes: Vec<TagNode>,
    /// The colour transform `decode_samples` left to the output pass.
    pub mct: Mct,
}

/// The working memory `Tile::decode_samples` takes from its decoder.
pub(crate) struct SampleWork<'a> {
    pub t1: &'a mut T1,
    pub dwt: &'a mut DwtScratch,
    pub planes: &'a mut [Vec<i32>],
    pub blocks: &'a mut Vec<BlockRef>,
}

/// OpenJPEG's struct sizes on 64-bit platforms, which its "system limits"
/// checks divide by: `sizeof(opj_tcd_precinct_t)` and
/// `sizeof(opj_tcd_cblk_dec_t)`.
const OPJ_PRECINCT_SIZE: u32 = 56;
const OPJ_CBLK_DEC_SIZE: u32 = 88;

fn int_ceildiv(a: i32, b: i32) -> i32 {
    ((i64::from(a) + i64::from(b) - 1) / i64::from(b)) as i32
}

fn int_ceildivpow2(a: i32, b: u32) -> i32 {
    ((i64::from(a) + (1i64 << b) - 1) >> b) as i32
}

fn int64_ceildivpow2(a: i64, b: u32) -> i32 {
    ((a + (1i64 << b) - 1) >> b) as i32
}

/// `opj_uint_adds`: the sum, saturated.
pub(crate) fn uint_adds(a: u32, b: u32) -> u32 {
    a.saturating_add(b)
}

/// `opj_tcd_is_subband_area_of_interest` for a whole-tile decode: whether
/// the band region `[x0, x1) x [y0, y1)` of resolution `resno` (band 0 LL, 1
/// HL, 2 LH, 3 HH) meets the tile-component, widened by the filter's margin,
/// as OpenJPEG computes it. Tier-2 (which packets to read) and Tier-1 (which
/// code-blocks to decode) take the same answer.
pub(crate) fn area_of_interest(tilec: &TileComp, resno: u32, bandno: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> bool {
    let filter_margin: u32 = if tilec.reversible { 2 } else { 3 };
    let nb = if resno == 0 {
        tilec.numresolutions - 1
    } else {
        tilec.numresolutions - resno
    };
    // a tile-component coordinate in the band's (ISO 15444-1 B-15)
    let band = |coord: i32, b: u32| -> u32 {
        let t = coord as u32;
        if nb == 0 {
            t
        } else {
            let half = (1u32 << (nb - 1)).wrapping_mul(b);
            if t <= half { 0 } else { ceildivpow2(t - half, nb) }
        }
    };
    let (xb, yb) = (bandno & 1, bandno >> 1);
    let tbx0 = band(tilec.x0, xb).saturating_sub(filter_margin);
    let tby0 = band(tilec.y0, yb).saturating_sub(filter_margin);
    let tbx1 = uint_adds(band(tilec.x1, xb), filter_margin);
    let tby1 = uint_adds(band(tilec.y1, yb), filter_margin);
    x0 < tbx1 && y0 < tby1 && x1 > tbx0 && y1 > tby0
}

/// 2^n as a double, exactly what `ldexp(1.0, n)` gives, for n in the normal
/// range; `Rb - expn` lies in -30 to 40 here.
fn pow2(n: i32) -> f64 {
    f64::from_bits(((n + 1023) as u64) << 52)
}

/// One code-block Tier-1 decodes: where it is in the layout and where its
/// samples go.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BlockRef {
    pub comp: u16,
    /// The band: 0 LL, 1 HL, 2 LH, 3 HH.
    pub orient: u8,
    pub w: u16,
    pub h: u16,
    /// Its entry in `Tile::cblks`.
    pub cblk: u32,
    /// Its first sample in the component's buffer, and the samples between
    /// two of its rows there (`dwt::RowLayout`).
    pub dst: usize,
    pub stride: usize,
    pub stepsize: f32,
}

impl Tile {
    /// `opj_tcd_init_tile` for decoding. `decoded` components have their
    /// samples decoded, the others only their packets read.
    pub fn init(
        &mut self,
        mh: &MainHeader,
        tile: u32,
        params: &[TileComponentParams<'_>],
        reduce: u32,
        decoded: usize,
    ) -> Result<(), Error> {
        let (p, q) = (tile % mh.tw, tile / mh.tw);
        // the four borders of the tile, clipped to the image
        let l_tx0 = mh.tx0.wrapping_add(p.wrapping_mul(mh.tdx));
        self.x0 = l_tx0.max(mh.x0) as i32;
        self.x1 = uint_adds(l_tx0, mh.tdx).min(mh.x1) as i32;
        if self.x0 < 0 || self.x1 <= self.x0 {
            return Err(Error::Unsupported("Tile X coordinates are not supported"));
        }
        let l_ty0 = mh.ty0.wrapping_add(q.wrapping_mul(mh.tdy));
        self.y0 = l_ty0.max(mh.y0) as i32;
        self.y1 = uint_adds(l_ty0, mh.tdy).min(mh.y1) as i32;
        if self.y0 < 0 || self.y1 <= self.y0 {
            return Err(Error::Unsupported("Tile Y coordinates are not supported"));
        }
        if params.iter().any(|p| p.style.cblksty & CBLK_HT != 0) {
            return Err(Error::Unsupported("HT code-blocks (JPEG 2000 Part 15) are not supported"));
        }

        self.comps.clear();
        self.res.clear();
        self.precincts.clear();
        self.tag_nodes.clear();
        self.mct = Mct::None;
        let mut ncblks = 0usize;
        for (compno, (sc, params)) in mh.comps.iter().zip(params).enumerate() {
            let style = params.style;
            let mut tc = TileComp {
                x0: int_ceildiv(self.x0, i32::from(sc.dx)),
                y0: int_ceildiv(self.y0, i32::from(sc.dy)),
                x1: int_ceildiv(self.x1, i32::from(sc.dx)),
                y1: int_ceildiv(self.y1, i32::from(sc.dy)),
                numresolutions: u32::from(style.numresolutions),
                res: self.res.len() as u32,
                cblksty: style.cblksty,
                reversible: style.qmfbid == 1,
                roishift: params.roishift,
                decoded: compno < decoded,
                ..TileComp::default()
            };
            if tc.numresolutions <= reduce {
                // the walk refuses such a tile before it gets here; its
                // decoded level would not be the image's
                return Err(Error::Limit("the reduce factor is not below the resolutions of a component"));
            }
            tc.minimum_num_resolutions = tc.numresolutions - reduce;
            tc.dc_shift = if sc.signed { 0 } else { (1u32 << (sc.prec - 1)) as i32 };
            let mut level_no = tc.numresolutions;
            for resno in 0..tc.numresolutions {
                level_no -= 1;
                let lv = level_no;
                let mut r = Resolution {
                    x0: int_ceildivpow2(tc.x0, lv),
                    y0: int_ceildivpow2(tc.y0, lv),
                    x1: int_ceildivpow2(tc.x1, lv),
                    y1: int_ceildivpow2(tc.y1, lv),
                    ..Resolution::default()
                };
                let pdx = u32::from(style.precincts[resno as usize] & 0xF);
                let pdy = u32::from(style.precincts[resno as usize] >> 4);
                r.pdx = pdx;
                r.pdy = pdy;
                let tl_prc_x_start = (r.x0 >> pdx) << pdx;
                let tl_prc_y_start = (r.y0 >> pdy) << pdy;
                let br_x = (int_ceildivpow2(r.x1, pdx) as u32) << pdx;
                let br_y = (int_ceildivpow2(r.y1, pdy) as u32) << pdy;
                if br_x > i32::MAX as u32 || br_y > i32::MAX as u32 {
                    return Err(Error::Unsupported("Integer overflow"));
                }
                let (br_prc_x_end, br_prc_y_end) = (br_x as i32, br_y as i32);
                r.pw = if r.x0 == r.x1 {
                    0
                } else {
                    ((br_prc_x_end - tl_prc_x_start) >> pdx) as u32
                };
                r.ph = if r.y0 == r.y1 {
                    0
                } else {
                    ((br_prc_y_end - tl_prc_y_start) >> pdy) as u32
                };
                if r.pw != 0 && u32::MAX / r.pw < r.ph {
                    return Err(Error::Limit("Size of tile data exceeds system limits"));
                }
                let nb_precincts = r.pw * r.ph;
                if u32::MAX / OPJ_PRECINCT_SIZE < nb_precincts {
                    return Err(Error::Limit("Size of tile data exceeds system limits"));
                }
                let (tlcbgxstart, tlcbgystart, cbgwidthexpn, cbgheightexpn);
                if resno == 0 {
                    tlcbgxstart = tl_prc_x_start;
                    tlcbgystart = tl_prc_y_start;
                    cbgwidthexpn = pdx;
                    cbgheightexpn = pdy;
                    r.numbands = 1;
                } else {
                    tlcbgxstart = int_ceildivpow2(tl_prc_x_start, 1);
                    tlcbgystart = int_ceildivpow2(tl_prc_y_start, 1);
                    // (a precinct exponent of 0 above resolution 0 is refused by the header)
                    cbgwidthexpn = pdx.saturating_sub(1);
                    cbgheightexpn = pdy.saturating_sub(1);
                    r.numbands = 3;
                }
                let cblkwidthexpn = u32::from(style.cblkw).min(cbgwidthexpn);
                let cblkheightexpn = u32::from(style.cblkh).min(cbgheightexpn);
                r.cblkw_expn = cblkwidthexpn;
                r.cblkh_expn = cblkheightexpn;
                for bandno in 0..r.numbands {
                    let mut band = Band::default();
                    if resno == 0 {
                        band.bandno = 0;
                        band.x0 = int_ceildivpow2(tc.x0, lv);
                        band.y0 = int_ceildivpow2(tc.y0, lv);
                        band.x1 = int_ceildivpow2(tc.x1, lv);
                        band.y1 = int_ceildivpow2(tc.y1, lv);
                    } else {
                        band.bandno = bandno + 1;
                        let x0b = i64::from(band.bandno & 1);
                        let y0b = i64::from(band.bandno >> 1);
                        band.x0 = int64_ceildivpow2(i64::from(tc.x0) - (x0b << level_no), lv + 1);
                        band.y0 = int64_ceildivpow2(i64::from(tc.y0) - (y0b << level_no), lv + 1);
                        band.x1 = int64_ceildivpow2(i64::from(tc.x1) - (x0b << level_no), lv + 1);
                        band.y1 = int64_ceildivpow2(i64::from(tc.y1) - (y0b << level_no), lv + 1);
                    }
                    // Table E-1, sub-band gains; 0 for irreversible bands, which
                    // the 9/7 inverse compensates with 2 / invK (dwt.rs)
                    let log2_gain = if !tc.reversible || band.bandno == 0 {
                        0
                    } else if band.bandno == 3 {
                        2
                    } else {
                        1
                    };
                    let rb = i32::from(sc.prec) + log2_gain;
                    let step_index = if resno == 0 { 0 } else { 3 * resno - 2 + bandno };
                    let (expn, mant) = params.quant.step(step_index);
                    // Delta_b of equation E-3: pow(2.0, n) is exact, as ldexp is
                    band.stepsize = ((1.0 + f64::from(mant) / 2048.0) * pow2(rb - expn as i32)) as f32;
                    // Mb of equation E-2
                    band.numbps = expn as i32 + i32::from(params.quant.numgbits) - 1;
                    band.precincts = self.precincts.len() as u32;
                    for precno in 0..nb_precincts {
                        let cbgxstart = tlcbgxstart + (precno % r.pw) as i32 * (1 << cbgwidthexpn);
                        let cbgystart = tlcbgystart + (precno / r.pw) as i32 * (1 << cbgheightexpn);
                        let cbgxend = cbgxstart + (1 << cbgwidthexpn);
                        let cbgyend = cbgystart + (1 << cbgheightexpn);
                        let mut prc = Precinct {
                            x0: cbgxstart.max(band.x0),
                            y0: cbgystart.max(band.y0),
                            x1: cbgxend.min(band.x1),
                            y1: cbgyend.min(band.y1),
                            ..Precinct::default()
                        };
                        let tlcblkxstart = (prc.x0 >> cblkwidthexpn) << cblkwidthexpn;
                        let tlcblkystart = (prc.y0 >> cblkheightexpn) << cblkheightexpn;
                        let brcblkxend = int_ceildivpow2(prc.x1, cblkwidthexpn) << cblkwidthexpn;
                        let brcblkyend = int_ceildivpow2(prc.y1, cblkheightexpn) << cblkheightexpn;
                        prc.cw = ((brcblkxend - tlcblkxstart) >> cblkwidthexpn) as u32;
                        prc.ch = ((brcblkyend - tlcblkystart) >> cblkheightexpn) as u32;
                        prc.cblk_x0 = tlcblkxstart;
                        prc.cblk_y0 = tlcblkystart;
                        let nb_code_blocks = prc.cw.wrapping_mul(prc.ch);
                        if u64::from(prc.cw) * u64::from(prc.ch) > u64::from(u32::MAX / OPJ_CBLK_DEC_SIZE) {
                            return Err(Error::Limit("Size of code block data exceeds system limits"));
                        }
                        let nodes = TagTree::node_count(prc.cw, prc.ch);
                        if (self.tag_nodes.len() as u64 + 2 * nodes) > u64::from(u32::MAX)
                            || ncblks as u64 + u64::from(nb_code_blocks) > u64::from(u32::MAX)
                        {
                            return Err(Error::Limit("Size of tile data exceeds system limits"));
                        }
                        prc.incl = TagTree {
                            off: self.tag_nodes.len() as u32,
                            w: prc.cw,
                            h: prc.ch,
                        };
                        prc.imsb = TagTree {
                            off: self.tag_nodes.len() as u32 + nodes as u32,
                            w: prc.cw,
                            h: prc.ch,
                        };
                        self.tag_nodes.resize(self.tag_nodes.len() + 2 * nodes as usize, TagNode::default());
                        prc.cblks = ncblks as u32;
                        ncblks += nb_code_blocks as usize;
                        self.precincts.push(prc);
                    }
                    r.bands[bandno as usize] = band;
                }
                self.res.push(r);
            }
            let dec = &self.res[(tc.res + tc.minimum_num_resolutions - 1) as usize];
            tc.data_w = (dec.x1 - dec.x0) as u32;
            tc.data_h = (dec.y1 - dec.y0) as u32;
            self.comps.push(tc);
        }
        // the code-blocks, each back to its first state with its lists' memory kept
        let kept = self.cblks.len().min(ncblks);
        for cb in &mut self.cblks[..kept] {
            cb.reset();
        }
        self.cblks.resize_with(ncblks, CodeBlock::default);
        Ok(())
    }

    /// The resolutions of component `c`.
    pub fn res_of(&self, c: usize) -> &[Resolution] {
        let tc = &self.comps[c];
        &self.res[tc.res as usize..(tc.res + tc.numresolutions) as usize]
    }

    /// Samples of the decoded resolution of component `c`.
    pub fn samples(&self, c: usize) -> usize {
        let tc = &self.comps[c];
        tc.data_w as usize * tc.data_h as usize
    }

    /// The resolutions of component `c` up to the one decoded, as the
    /// wavelet takes them.
    fn rects(&self, c: usize, rects: &mut [ResRect; MAX_RESOLUTIONS]) -> usize {
        let tc = &self.comps[c];
        let res = &self.res_of(c)[..tc.minimum_num_resolutions as usize];
        for (rect, r) in rects.iter_mut().zip(res) {
            *rect = ResRect {
                x0: r.x0,
                y0: r.y0,
                x1: r.x1,
                y1: r.y1,
            };
        }
        res.len()
    }

    /// Every code-block Tier-1 decodes, in OpenJPEG's order
    /// (`opj_t1_decode_cblks` for a whole tile, component by component),
    /// each with its place in its component's buffer: OpenJPEG's column,
    /// and the row the wavelet wants it on (`dwt::RowLayout`).
    fn collect_blocks(&self, out: &mut Vec<BlockRef>) {
        out.clear();
        let mut rects = [ResRect::default(); MAX_RESOLUTIONS];
        for (compno, tc) in self.comps.iter().enumerate() {
            if !tc.decoded {
                continue;
            }
            let tile_w = tc.data_w as usize;
            let res = self.res_of(compno);
            let numres = self.rects(compno, &mut rects);
            let layout = RowLayout::new(&rects[..numres]);
            for resno in 0..tc.minimum_num_resolutions {
                let r = &res[resno as usize];
                for band in &r.bands[..r.numbands as usize] {
                    let np = r.pw * r.ph;
                    for precno in 0..np {
                        let prc = &self.precincts[(band.precincts + precno) as usize];
                        if !area_of_interest(tc, resno, band.bandno, prc.x0 as u32, prc.y0 as u32, prc.x1 as u32, prc.y1 as u32) {
                            continue;
                        }
                        let nb = prc.cw * prc.ch;
                        for cblkno in 0..nb {
                            let cbx = prc.cblk_x0 + (cblkno % prc.cw) as i32 * (1 << r.cblkw_expn);
                            let cby = prc.cblk_y0 + (cblkno / prc.cw) as i32 * (1 << r.cblkh_expn);
                            let (cx0, cy0) = (cbx.max(prc.x0), cby.max(prc.y0));
                            let cx1 = (cbx + (1 << r.cblkw_expn)).min(prc.x1);
                            let cy1 = (cby + (1 << r.cblkh_expn)).min(prc.y1);
                            if !area_of_interest(tc, resno, band.bandno, cx0 as u32, cy0 as u32, cx1 as u32, cy1 as u32) {
                                continue;
                            }
                            if cx1 <= cx0 || cy1 <= cy0 {
                                continue;
                            }
                            let mut x = cx0 - band.x0;
                            if band.bandno & 1 != 0 {
                                let pres = &res[resno as usize - 1];
                                x += pres.x1 - pres.x0;
                            }
                            let y = (cy0 - band.y0) as usize;
                            let (row, rows) = layout.band_row(resno as usize, band.bandno & 2 != 0, (r.y0 & 1) as usize, y);
                            out.push(BlockRef {
                                comp: compno as u16,
                                orient: band.bandno as u8,
                                w: (cx1 - cx0) as u16,
                                h: (cy1 - cy0) as u16,
                                cblk: prc.cblks + cblkno,
                                dst: row * tile_w + x as usize,
                                stride: rows * tile_w,
                                stepsize: band.stepsize,
                            });
                        }
                    }
                }
            }
        }
    }

    /// Tier-1 into each component's buffer (`planes[c]`: the decoded
    /// resolution's samples), the inverse wavelet up to `resno_decoded`, and
    /// the colour transform's checks (`opj_tcd_decode_tile` after Tier-2):
    /// `self.mct` says which transform the samples still need. The transform
    /// and the DC shift ([`Tile::finish_of`]) are the output pass's.
    pub fn decode_samples(&mut self, coding_mct: bool, data: &[u8], resno_decoded: &[u32], work: SampleWork<'_>) -> Result<(), Error> {
        // Tier-1 (opj_t1_decode_cblks, whole tile), component by component
        self.collect_blocks(work.blocks);
        self.decode_blocks(work.blocks, data, work.t1, work.planes)?;

        // the inverse wavelet up to the resolution decoded (opj_tcd_dwt_decode)
        let mut rects = [ResRect::default(); MAX_RESOLUTIONS];
        for (compno, tc) in self.comps.iter().enumerate() {
            if !tc.decoded {
                continue;
            }
            let top = self.rects(compno, &mut rects);
            let numres = resno_decoded[compno] as usize + 1;
            dwt::decode(
                &mut work.planes[compno],
                tc.data_w as usize,
                &rects[..top],
                numres,
                tc.reversible,
                work.dwt,
            );
        }

        // the colour transform (opj_tcd_mct_decode)
        if coding_mct {
            let numcomps = self.comps.len();
            if numcomps >= 3 {
                let c0 = &self.comps[0];
                if c0.minimum_num_resolutions != self.comps[1].minimum_num_resolutions
                    || c0.minimum_num_resolutions != self.comps[2].minimum_num_resolutions
                {
                    return Err(Error::Data("Tiles don't all have the same dimension"));
                }
                let last = c0.minimum_num_resolutions as usize - 1;
                let area = |c: usize| {
                    let r = &self.res_of(c)[last];
                    (r.x1 - r.x0) as u32 as usize * (r.y1 - r.y0) as u32 as usize
                };
                let l_samples = area(0);
                if resno_decoded[0] != resno_decoded[1]
                    || resno_decoded[0] != resno_decoded[2]
                    || area(1) != l_samples
                    || area(2) != l_samples
                {
                    return Err(Error::Data("Tiles don't all have the same dimension"));
                }
                self.mct = if c0.reversible { Mct::Rct } else { Mct::Ict };
            }
            // fewer than three components: OpenJPEG says so and skips the transform
        }
        Ok(())
    }

    /// Tier-1 of the whole walk, in its order; an error at a block OpenJPEG
    /// refuses, as `opj_t1_decode_cblks` stops there.
    fn decode_blocks(&self, blocks: &[BlockRef], data: &[u8], t1: &mut T1, planes: &mut [Vec<i32>]) -> Result<(), Error> {
        for b in blocks {
            let tc = &self.comps[usize::from(b.comp)];
            let cb = &self.cblks[b.cblk as usize];
            let block = T1Block {
                w: u32::from(b.w),
                h: u32::from(b.h),
                numbps: cb.numbps,
                orient: u32::from(b.orient),
                roishift: u32::from(tc.roishift),
                cblksty: u32::from(tc.cblksty),
                chunks: &cb.chunks,
                segs: &cb.segs[..(cb.real_num_segs as usize).min(cb.segs.len())],
            };
            let dst = &mut planes[usize::from(b.comp)][b.dst..];
            // one code-block decoded (zeros without data) and stored into
            // its component's buffer
            if block.chunks.is_empty() {
                if !t1::empty_cblk_ok(&block) {
                    return Err(Error::Data("code-block with too many bit-planes"));
                }
                t1::store(None, block.w, block.h, tc.reversible, b.stepsize, dst, b.stride);
                continue;
            }
            if !t1.decode_cblk(&block, data) {
                return Err(Error::Data("code-block with too many bit-planes"));
            }
            t1.unshift_roi(block.roishift);
            t1::store(Some(t1.data()), block.w, block.h, tc.reversible, b.stepsize, dst, b.stride);
        }
        Ok(())
    }

    /// Component `compno`'s DC shift (`opj_tcd_dc_level_shift_decode`'s
    /// parameters).
    pub fn finish_of(&self, mh: &MainHeader, compno: usize) -> Finish {
        let sc = &mh.comps[compno];
        let tc = &self.comps[compno];
        let (lo, hi) = if sc.signed {
            (-((1u32 << (sc.prec - 1)) as i32), ((1u32 << (sc.prec - 1)) - 1) as i32)
        } else {
            (0, ((1u32 << sc.prec) - 1) as i32)
        };
        Finish {
            shift: tc.dc_shift,
            lo,
            hi,
            reversible: tc.reversible,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_tree_node_counts() {
        assert_eq!(TagTree::node_count(0, 5), 0);
        assert_eq!(TagTree::node_count(1, 1), 1);
        assert_eq!(TagTree::node_count(2, 2), 5);
        assert_eq!(TagTree::node_count(3, 2), 6 + 2 + 1);
        assert_eq!(TagTree::node_count(16, 16), 256 + 64 + 16 + 4 + 1);
    }

    #[test]
    fn pow2_is_ldexp() {
        assert_eq!(pow2(0), 1.0);
        assert_eq!(pow2(10), 1024.0);
        assert_eq!(pow2(-3), 0.125);
    }
}
