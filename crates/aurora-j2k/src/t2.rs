// This file is a port and keeps the notices of what it ports (both under the
// BSD 2-Clause License, whose text NOTICE.md gives, "JPEG 2000 decoder").
//
// neo-j2k, `src/t2.cpp`:
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

//! Tier-2 decoding of a tile: the packet iterator, the bit reader of packet
//! headers, the tag trees and the packet reader.
//!
//! Port of neo-j2k's `src/t2.cpp` (BSD-2-Clause, Copyright (c) 2026 Anne
//! Skydancer), itself after OpenJPEG 2.5.4's `pi.c` (`opj_pi_create_decode`
//! and the five progression orders, POC included), `bio.c`, `tgt.c` and
//! `t2.c` (`opj_t2_decode_packets`, `opj_t2_read_packet_header`,
//! `opj_t2_read_packet_data`, `opj_t2_skip_packet_data`; BSD-2-Clause, see
//! NOTICE.md), with the same order and the same arithmetic, so every packet
//! header is read as OpenJPEG reads it.
//!
//! Cut codestreams (a texture whose download stopped at some byte) follow
//! neo-j2k, which keeps every byte that arrived:
//! - a packet header the data ends in is an absent packet;
//! - a contribution the data ends in is left out, with the contributions
//!   after it in the packet; what earlier packets gave those code-blocks
//!   stays (OpenJPEG 2.5.3 drops such a code-block whole);
//! - packets are no longer visited once the tile's data has run out.
//!
//! [`PacketStats::cut_in_decoded`] says when a decode took the second rule
//! in a resolution it decodes: the one case where its samples can differ
//! from OpenJPEG's.

use std::ops::ControlFlow;

use crate::Error;
use crate::header::{CSTY_EPH, CSTY_SOP, MainHeader, PocEntry, TileComponentParams, ceildiv};
use crate::tile::{Chunk, CodeBlock, Precinct, Resolution, Segment, TagNode, TagTree, Tile, area_of_interest, uint_adds};

pub(crate) const CBLK_LAZY: u32 = 0x01;
pub(crate) const CBLK_TERMALL: u32 = 0x04;

const LRCP: u8 = 0;
const RLCP: u8 = 1;
const RPCL: u8 = 2;
const PCRL: u8 = 3;
const CPRL: u8 = 4;

fn uint_floorlog2(mut a: u32) -> u32 {
    let mut l = 0;
    while a > 1 {
        a >>= 1;
        l += 1;
    }
    l
}

fn uint_ceildivpow2(a: u32, b: u32) -> u32 {
    ((u64::from(a) + (1u64 << b) - 1) >> b) as u32
}

fn ceildiv64(a: u64, b: u64) -> u32 {
    a.div_ceil(b) as u32
}

// ---- bio.c: the bit reader of packet headers ---------------------------------------------------

struct Bio<'a> {
    data: &'a [u8],
    bp: usize,
    buf: u32,
    ct: u32,
    /// A byte was asked for past the end of the data.
    overrun: bool,
}

impl<'a> Bio<'a> {
    fn new(data: &'a [u8]) -> Self {
        Bio {
            data,
            bp: 0,
            buf: 0,
            ct: 0,
            overrun: false,
        }
    }

    fn bytein(&mut self) -> bool {
        self.buf = (self.buf << 8) & 0xffff;
        self.ct = if self.buf == 0xff00 { 7 } else { 8 };
        let Some(&b) = self.data.get(self.bp) else {
            self.overrun = true;
            return false;
        };
        self.buf |= u32::from(b);
        self.bp += 1;
        true
    }

    fn getbit(&mut self) -> u32 {
        if self.ct == 0 {
            self.bytein();
        }
        self.ct -= 1;
        (self.buf >> self.ct) & 1
    }

    fn read(&mut self, n: u32) -> u32 {
        let mut v = 0;
        for i in (0..n).rev() {
            v |= self.getbit() << i;
        }
        v
    }

    fn inalign(&mut self) -> bool {
        if self.buf & 0xff == 0xff && !self.bytein() {
            return false;
        }
        self.ct = 0;
        true
    }

    fn numbytes(&self) -> usize {
        self.bp
    }
}

fn getcommacode(bio: &mut Bio<'_>) -> u32 {
    let mut n = 0;
    while bio.read(1) != 0 {
        n += 1;
    }
    n
}

fn getnumpasses(bio: &mut Bio<'_>) -> u32 {
    if bio.read(1) == 0 {
        return 1;
    }
    if bio.read(1) == 0 {
        return 2;
    }
    let n = bio.read(2);
    if n != 3 {
        return 3 + n;
    }
    let n = bio.read(5);
    if n != 31 {
        return 6 + n;
    }
    37 + bio.read(7)
}

// ---- tgt.c: tag trees --------------------------------------------------------------------------

fn tgt_reset(nodes: &mut [TagNode], t: &TagTree) {
    let n = TagTree::node_count(t.w, t.h) as usize;
    for node in &mut nodes[t.off as usize..t.off as usize + n] {
        *node = TagNode { value: 999, low: 0 };
    }
}

/// `opj_tgt_decode`: the path from the root down to the leaf, each node's
/// value read until the threshold is reached.
fn tgt_decode(bio: &mut Bio<'_>, nodes: &mut [TagNode], t: &TagTree, leafno: u32, threshold: i32) -> bool {
    let mut stk = [0usize; 32];
    let mut depth = 0;
    let (mut w, mut h) = (t.w as usize, t.h as usize);
    let (mut x, mut y) = ((leafno % t.w) as usize, (leafno / t.w) as usize);
    let mut base = t.off as usize;
    loop {
        stk[depth] = base + y * w + x;
        depth += 1;
        let n = w * h;
        if n <= 1 {
            break;
        }
        base += n;
        w = w.div_ceil(2);
        h = h.div_ceil(2);
        x >>= 1;
        y >>= 1;
    }
    // stk[0] is the leaf, stk[depth - 1] the root
    let mut low = 0i32;
    loop {
        depth -= 1;
        let node = &mut nodes[stk[depth]];
        if low > node.low {
            node.low = low;
        } else {
            low = node.low;
        }
        while low < threshold && low < node.value {
            if bio.read(1) != 0 {
                node.value = low;
            } else {
                low += 1;
            }
        }
        node.low = low;
        if depth == 0 {
            return node.value < threshold;
        }
    }
}

// ---- pi.c: the packet iterator -----------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct PiResolution {
    pdx: u32,
    pdy: u32,
    pw: u32,
    ph: u32,
}

struct PiComp {
    dx: u32,
    dy: u32,
    /// Its resolutions in `PiLayout::resolutions`.
    first: usize,
    numresolutions: u32,
}

/// One progression (`opj_poc_t` as the decoder fills it).
#[derive(Clone, Copy)]
struct Progression {
    prg: u8,
    resno0: u32,
    compno0: u32,
    resno1: u32,
    compno1: u32,
    layno1: u32,
}

/// What the packet iterators of a tile share.
struct PiLayout {
    comps: Vec<PiComp>,
    resolutions: Vec<PiResolution>,
    tx0: u32,
    ty0: u32,
    tx1: u32,
    ty1: u32,
    step_l: u64,
    step_r: u64,
    step_c: u64,
}

impl PiLayout {
    fn res(&self, comp: &PiComp, resno: u32) -> &PiResolution {
        &self.resolutions[comp.first + resno as usize]
    }
}

/// Packets already met (`opj_pi_iterator_t::include`, as bits): one per
/// layer (numlayers + 1), resolution, component and precinct.
struct Included<'a> {
    bits: &'a mut [u64],
    size: u64,
}

/// A packet's place.
#[derive(Clone, Copy)]
struct PacketPos {
    compno: u32,
    resno: u32,
    precno: u32,
    layno: u32,
}

impl Included<'_> {
    /// `Some(true)` when the packet is new (and is marked), `Some(false)`
    /// when met before; `None` for an index past the table, which ends the
    /// iterator ("Invalid access to pi->include").
    fn include(&mut self, layout: &PiLayout, p: PacketPos) -> Option<bool> {
        let index = u64::from(p.layno) * layout.step_l
            + u64::from(p.resno) * layout.step_r
            + u64::from(p.compno) * layout.step_c
            + u64::from(p.precno);
        if index >= self.size {
            return None;
        }
        let w = &mut self.bits[(index >> 6) as usize];
        let m = 1u64 << (index & 63);
        let was = *w & m != 0;
        *w |= m;
        Some(!was)
    }
}

/// The precinct grid step over a component's resolutions, as
/// `opj_pi_next_rpcl` and its siblings compute it.
fn pi_min_steps(layout: &PiLayout, comp: &PiComp, pdx: &mut u32, pdy: &mut u32) {
    for resno in 0..comp.numresolutions {
        let r = layout.res(comp, resno);
        let ex = r.pdx + comp.numresolutions - 1 - resno;
        let ey = r.pdy + comp.numresolutions - 1 - resno;
        if ex < 32 && comp.dx <= u32::MAX / (1u32 << ex) {
            let dx = comp.dx * (1u32 << ex);
            *pdx = if *pdx == 0 { dx } else { (*pdx).min(dx) };
        }
        if ey < 32 && comp.dy <= u32::MAX / (1u32 << ey) {
            let dy = comp.dy * (1u32 << ey);
            *pdy = if *pdy == 0 { dy } else { (*pdy).min(dy) };
        }
    }
}

/// The precinct of `comp` at resolution `resno` that the position `(x, y)`
/// starts, as OpenJPEG decides it for the three position-driven orders;
/// `None` when the position starts none.
fn pi_position_precinct(layout: &PiLayout, comp: &PiComp, resno: u32, x: u32, y: u32) -> Option<u32> {
    let res = layout.res(comp, resno);
    let levelno = comp.numresolutions - 1 - resno;
    let (dx, dy) = (u64::from(comp.dx), u64::from(comp.dy));
    if ((dx << levelno) >> levelno) as u32 != comp.dx || ((dy << levelno) >> levelno) as u32 != comp.dy {
        return None;
    }
    // (levelno is below 33 and dx below 256: the shifts fit 64 bits)
    let trx0 = ceildiv64(u64::from(layout.tx0), dx << levelno);
    let try0 = ceildiv64(u64::from(layout.ty0), dy << levelno);
    let trx1 = ceildiv64(u64::from(layout.tx1), dx << levelno);
    let try1 = ceildiv64(u64::from(layout.ty1), dy << levelno);
    let rpx = res.pdx + levelno;
    let rpy = res.pdy + levelno;
    if ((dx << rpx) >> rpx) as u32 != comp.dx || ((dy << rpy) >> rpy) as u32 != comp.dy {
        return None;
    }
    // ISO 15444-1 B.12.1.3 to B.12.1.5
    if !(u64::from(y) % (dy << rpy) == 0 || (y == layout.ty0 && (u64::from(try0) << levelno) % (1u64 << rpy) != 0)) {
        return None;
    }
    if !(u64::from(x) % (dx << rpx) == 0 || (x == layout.tx0 && (u64::from(trx0) << levelno) % (1u64 << rpx) != 0)) {
        return None;
    }
    if res.pw == 0 || res.ph == 0 {
        return None;
    }
    if trx0 == trx1 || try0 == try1 {
        return None;
    }
    let prci = (ceildiv64(u64::from(x), dx << levelno) >> res.pdx).wrapping_sub(trx0 >> res.pdx);
    let prcj = (ceildiv64(u64::from(y), dy << levelno) >> res.pdy).wrapping_sub(try0 >> res.pdy);
    Some(prci.wrapping_add(prcj.wrapping_mul(res.pw)))
}

/// The positions of a position-driven order along one axis: `t0`, then each
/// multiple of `d` below `t1` (`for (x = tx0; x < tx1; x += dx - (x % dx))`).
fn positions(t0: u32, t1: u32, d: u32) -> impl Iterator<Item = u32> {
    let mut x = u64::from(t0);
    let (t1, d) = (u64::from(t1), u64::from(d));
    std::iter::from_fn(move || {
        if x >= t1 {
            return None;
        }
        let at = x as u32;
        x += d - (x % d);
        Some(at)
    })
}

/// Every packet of one progression, in its order (`opj_pi_next` until it
/// gives no more): `visit` is called for each packet not met before, and
/// stops the walk by breaking.
fn for_each_packet<B>(
    layout: &PiLayout,
    included: &mut Included<'_>,
    poc: &Progression,
    mut visit: impl FnMut(PacketPos) -> ControlFlow<B>,
) -> ControlFlow<B> {
    let numcomps = layout.comps.len() as u32;
    if poc.compno0 >= numcomps || poc.compno1 > numcomps {
        // "invalid compno0/compno1"
        return ControlFlow::Continue(());
    }
    // the include table; an index past it ends the iterator
    macro_rules! packet {
        ($p:expr) => {
            match included.include(layout, $p) {
                None => return ControlFlow::Continue(()),
                Some(true) => visit($p)?,
                Some(false) => {}
            }
        };
    }
    match poc.prg {
        LRCP => {
            for layno in 0..poc.layno1 {
                for resno in poc.resno0..poc.resno1 {
                    for compno in poc.compno0..poc.compno1 {
                        let comp = &layout.comps[compno as usize];
                        if resno >= comp.numresolutions {
                            continue;
                        }
                        let res = layout.res(comp, resno);
                        for precno in 0..res.pw.wrapping_mul(res.ph) {
                            packet!(PacketPos {
                                compno,
                                resno,
                                precno,
                                layno
                            });
                        }
                    }
                }
            }
        }
        RLCP => {
            for resno in poc.resno0..poc.resno1 {
                for layno in 0..poc.layno1 {
                    for compno in poc.compno0..poc.compno1 {
                        let comp = &layout.comps[compno as usize];
                        if resno >= comp.numresolutions {
                            continue;
                        }
                        let res = layout.res(comp, resno);
                        for precno in 0..res.pw.wrapping_mul(res.ph) {
                            packet!(PacketPos {
                                compno,
                                resno,
                                precno,
                                layno
                            });
                        }
                    }
                }
            }
        }
        RPCL | PCRL => {
            let (mut dx, mut dy) = (0, 0);
            for comp in &layout.comps {
                pi_min_steps(layout, comp, &mut dx, &mut dy);
            }
            if dx == 0 || dy == 0 {
                return ControlFlow::Continue(());
            }
            if poc.prg == RPCL {
                for resno in poc.resno0..poc.resno1 {
                    for y in positions(layout.ty0, layout.ty1, dy) {
                        for x in positions(layout.tx0, layout.tx1, dx) {
                            for compno in poc.compno0..poc.compno1 {
                                let comp = &layout.comps[compno as usize];
                                if resno >= comp.numresolutions {
                                    continue;
                                }
                                let Some(precno) = pi_position_precinct(layout, comp, resno, x, y) else {
                                    continue;
                                };
                                for layno in 0..poc.layno1 {
                                    packet!(PacketPos {
                                        compno,
                                        resno,
                                        precno,
                                        layno
                                    });
                                }
                            }
                        }
                    }
                }
            } else {
                for y in positions(layout.ty0, layout.ty1, dy) {
                    for x in positions(layout.tx0, layout.tx1, dx) {
                        for compno in poc.compno0..poc.compno1 {
                            let comp = &layout.comps[compno as usize];
                            for resno in poc.resno0..poc.resno1.min(comp.numresolutions) {
                                let Some(precno) = pi_position_precinct(layout, comp, resno, x, y) else {
                                    continue;
                                };
                                for layno in 0..poc.layno1 {
                                    packet!(PacketPos {
                                        compno,
                                        resno,
                                        precno,
                                        layno
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        CPRL => {
            for compno in poc.compno0..poc.compno1 {
                let comp = &layout.comps[compno as usize];
                let (mut dx, mut dy) = (0, 0);
                pi_min_steps(layout, comp, &mut dx, &mut dy);
                if dx == 0 || dy == 0 {
                    return ControlFlow::Continue(());
                }
                for y in positions(layout.ty0, layout.ty1, dy) {
                    for x in positions(layout.tx0, layout.tx1, dx) {
                        for resno in poc.resno0..poc.resno1.min(comp.numresolutions) {
                            let Some(precno) = pi_position_precinct(layout, comp, resno, x, y) else {
                                continue;
                            };
                            for layno in 0..poc.layno1 {
                                packet!(PacketPos {
                                    compno,
                                    resno,
                                    precno,
                                    layno
                                });
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
    ControlFlow::Continue(())
}

// ---- t2.c: packets -----------------------------------------------------------------------------

/// Packed packet headers being consumed: PPM for the whole codestream, PPT
/// for a tile.
#[derive(Debug, Default)]
pub(crate) struct PackedCursor {
    pub pos: usize,
}

/// What a tile's packets told beyond its code-blocks.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct PacketStats {
    /// The tile's data ended inside a packet that is decoded, in its header
    /// after it had included code-blocks or in a contribution: those
    /// code-blocks keep what earlier packets gave them, where OpenJPEG
    /// 2.5.3 drops them whole.
    pub cut_in_decoded: bool,
}

/// Where a tile's packet headers come from.
pub(crate) enum PacketHeaders<'a> {
    /// In the tile's data, before each packet's body.
    Inline,
    /// PPM: the main header's, consumed across the tiles.
    Ppm(&'a [u8], &'a mut PackedCursor),
    /// PPT: the tile's own, merged in Zppt order.
    Ppt(&'a [u8], PackedCursor),
}

struct T2<'a, 'b> {
    res: &'a [Resolution],
    precincts: &'a [Precinct],
    cblks: &'a mut [CodeBlock],
    tag_nodes: &'a mut [TagNode],
    csty: u8,
    /// The bytes ended before the tile-parts' declared lengths.
    cut: bool,
    headers: PacketHeaders<'b>,
    /// The tile's bytes were cut inside a packet's SOP marker: OpenJPEG then
    /// reads the packet header from the marker's first bytes and fails, and
    /// so does this decoder; the failure is the cut's.
    cut_sop: bool,
    stats: PacketStats,
}

/// How a packet header's read ended.
enum Header {
    /// Read whole: whether the packet has data, and the header's bytes.
    Read { present: bool, used: usize },
    /// The data ended inside it: the packet is absent, and the rest of the
    /// tile's data and of the packet headers counts as read. `drop`: the
    /// passes the header had given so far are dropped.
    Cut { drop: bool },
}

/// `opj_t2_init_seg`
fn init_seg(cblk: &mut CodeBlock, index: usize, cblksty: u32, first: bool) {
    if cblk.segs.len() <= index {
        cblk.segs.resize(index + 1, Segment::default());
    }
    let prev_max = if index > 0 { cblk.segs[index - 1].maxpasses } else { 0 };
    let maxpasses = if cblksty & CBLK_TERMALL != 0 {
        1
    } else if cblksty & CBLK_LAZY != 0 {
        if first {
            10
        } else if prev_max == 1 || prev_max == 10 {
            2
        } else {
            1
        }
    } else {
        109
    };
    cblk.segs[index] = Segment {
        maxpasses,
        ..Segment::default()
    };
}

/// `opj_t2_first_new_seg`: the index of the first segment the packet's new
/// passes go to.
fn first_new_seg(cblk: &mut CodeBlock) -> usize {
    if cblk.numsegs == 0 {
        cblk.numsegs = 1;
        return 0;
    }
    let i = cblk.numsegs as usize - 1;
    match cblk.segs.get(i) {
        Some(seg) if seg.numpasses == seg.maxpasses => {
            cblk.numsegs += 1;
            i + 1
        }
        _ => i,
    }
}

/// `opj_t2_skip_cblk_passes`: the new passes counted in the header state,
/// without their bytes.
fn skip_cblk_passes(cblk: &mut CodeBlock, mut seg: usize) {
    loop {
        let Some(s) = cblk.segs.get_mut(seg) else {
            // (the packet header made a segment for every new pass)
            cblk.numnewpasses = 0;
            return;
        };
        s.numpasses += s.numnewpasses;
        cblk.numnewpasses = cblk.numnewpasses.saturating_sub(s.numnewpasses);
        if cblk.numnewpasses == 0 || s.numnewpasses == 0 {
            cblk.numnewpasses = 0;
            return;
        }
        seg += 1;
        cblk.numsegs += 1;
    }
}

impl T2<'_, '_> {
    /// The code-blocks of band `bandno` of the packet's precinct, with its
    /// precinct; `None` for an empty band.
    fn band_precinct(&self, res: &Resolution, bandno: usize, precno: u32) -> Option<Precinct> {
        let band = &res.bands[bandno];
        if band.empty() {
            return None;
        }
        self.precincts.get((band.precincts + precno) as usize).copied()
    }

    /// `opj_t2_drop_new_passes`: whether a code-block lost passes.
    fn drop_new_passes(&mut self, res: &Resolution, precno: u32) -> bool {
        let mut dropped = false;
        for bandno in 0..res.numbands as usize {
            let Some(prc) = self.band_precinct(res, bandno, precno) else {
                continue;
            };
            let n = (prc.cw * prc.ch) as usize;
            for cblk in &mut self.cblks[prc.cblks as usize..prc.cblks as usize + n] {
                if cblk.numnewpasses != 0 {
                    cblk.corrupted = true;
                    cblk.numnewpasses = 0;
                    dropped = true;
                }
            }
        }
        dropped
    }

    /// `opj_t2_read_packet_header`: whether the packet has data, and the
    /// bytes of the tile's data read. `src` is the tile's data from the
    /// packet on.
    fn read_packet_header(
        &mut self,
        res: &Resolution,
        cblksty: u32,
        pi: PacketPos,
        src: &[u8],
        skip: bool,
    ) -> Result<(bool, usize), Error> {
        let p_max_length = src.len();
        let mut current = 0usize;

        if pi.layno == 0 {
            // reset the tag trees
            for bandno in 0..res.numbands as usize {
                let Some(prc) = self.band_precinct(res, bandno, pi.precno) else {
                    continue;
                };
                tgt_reset(self.tag_nodes, &prc.incl);
                tgt_reset(self.tag_nodes, &prc.imsb);
                let n = (prc.cw * prc.ch) as usize;
                for cblk in &mut self.cblks[prc.cblks as usize..prc.cblks as usize + n] {
                    cblk.numsegs = 0;
                    cblk.real_num_segs = 0;
                }
            }
        }

        // SOP markers are optional (CVE-2023-39327)
        self.cut_sop = false;
        if self.csty & CSTY_SOP != 0 {
            if p_max_length < 6 {
                if self.cut && p_max_length > 0 {
                    // what is left starts a SOP marker (FF 91 00 04 Nsop): the bytes ended inside it
                    const SOP: [u8; 4] = [0xFF, 0x91, 0x00, 0x04];
                    let n = p_max_length.min(4);
                    self.cut_sop = src[..n] == SOP[..n];
                }
            } else if src[0] == 0xff && src[1] == 0x91 {
                current += 6;
            }
        }

        // the header's bytes: packed (PPM, PPT) or in the tile's data
        let header_src: &[u8] = match &self.headers {
            PacketHeaders::Inline => &src[current..],
            PacketHeaders::Ppm(data, cursor) => &data[cursor.pos.min(data.len())..],
            PacketHeaders::Ppt(data, cursor) => &data[cursor.pos.min(data.len())..],
        };
        let source_len = header_src.len();
        let mut bio = Bio::new(header_src);
        let outcome: Result<Header, Error> = (|| {
            let present = bio.read(1);
            if present == 0 {
                let cut = !bio.inalign() || bio.overrun;
                let mut used = bio.numbytes();
                if cut {
                    // the data ended inside this packet header: the packet is absent, and the
                    // rest of the tile's data and of the packet headers counts as read
                    return Ok(Header::Cut { drop: false });
                }
                if self.csty & CSTY_EPH != 0 {
                    if source_len - used < 2 {
                        return Ok(Header::Cut { drop: false });
                    } else if header_src[used] != 0xff || header_src[used + 1] != 0x92 {
                        return Err(Error::Data("Expected EPH marker"));
                    }
                    used += 2;
                }
                return Ok(Header::Read { present: false, used });
            }
            for bandno in 0..res.numbands as usize {
                let Some(prc) = self.band_precinct(res, bandno, pi.precno) else {
                    continue;
                };
                let band_numbps = res.bands[bandno].numbps;
                let n = prc.cw * prc.ch;
                for cblkno in 0..n {
                    let cblk = &mut self.cblks[(prc.cblks + cblkno) as usize];
                    // a code-block not yet included: the inclusion tag tree, else one bit
                    let included = if cblk.numsegs == 0 {
                        tgt_decode(&mut bio, self.tag_nodes, &prc.incl, cblkno, (pi.layno + 1) as i32)
                    } else {
                        bio.read(1) != 0
                    };
                    if !included {
                        cblk.numnewpasses = 0;
                        continue;
                    }
                    // a code-block not yet included: the zero bit-plane tag tree
                    if cblk.numsegs == 0 {
                        let mut i = 0u32;
                        while !tgt_decode(&mut bio, self.tag_nodes, &prc.imsb, cblkno, i as i32) {
                            i += 1;
                        }
                        // as OpenJPEG (pull request 1488): no overflow, a large count
                        cblk.numbps = (band_numbps as u32).wrapping_add(1).wrapping_sub(i);
                        cblk.numlenbits = 3;
                    }
                    cblk.numnewpasses = getnumpasses(&mut bio);
                    let increment = getcommacode(&mut bio);
                    cblk.numlenbits = cblk.numlenbits.wrapping_add(increment);
                    let mut segno = 0usize;
                    if cblk.numsegs == 0 {
                        init_seg(cblk, segno, cblksty, true);
                    } else {
                        segno = cblk.numsegs as usize - 1;
                        if cblk.segs.len() <= segno {
                            return Err(Error::Data("code-block segment state out of step"));
                        }
                        if cblk.segs[segno].numpasses == cblk.segs[segno].maxpasses {
                            segno += 1;
                            init_seg(cblk, segno, cblksty, false);
                        }
                    }
                    let mut left = cblk.numnewpasses as i32;
                    loop {
                        let numlenbits = cblk.numlenbits;
                        let s = &mut cblk.segs[segno];
                        let room = s.maxpasses.wrapping_sub(s.numpasses) as i32;
                        s.numnewpasses = room.min(left) as u32;
                        let bit_number = numlenbits.wrapping_add(uint_floorlog2(s.numnewpasses));
                        if bit_number > 32 {
                            return Err(Error::Data("Invalid bit number in a packet header"));
                        }
                        s.newlen = bio.read(bit_number);
                        left -= s.numnewpasses as i32;
                        if left <= 0 {
                            break;
                        }
                        if s.numnewpasses == 0 {
                            // (a segment without room: never from the rules above)
                            return Err(Error::Data("code-block segment state out of step"));
                        }
                        segno += 1;
                        init_seg(cblk, segno, cblksty, false);
                    }
                }
            }
            let aligned = bio.inalign();
            if !aligned || bio.overrun {
                // the data ended inside this packet header: the packet is absent, everything
                // earlier packets gave stays, and the rest of the tile's data and of the packet
                // headers counts as read
                return Ok(Header::Cut { drop: true });
            }
            let mut used = bio.numbytes();
            if self.csty & CSTY_EPH != 0 {
                if source_len - used < 2 {
                    return Ok(Header::Cut { drop: true });
                } else if header_src[used] != 0xff || header_src[used + 1] != 0x92 {
                    return Err(Error::Data("Expected EPH marker"));
                }
                used += 2;
            }
            if used == 0 {
                return Err(Error::Data("empty packet header"));
            }
            Ok(Header::Read { present: true, used })
        })();
        let (present, used) = match outcome? {
            Header::Read { present, used } => (present, used),
            Header::Cut { drop } => {
                // (OpenJPEG reads the rest of such a header as zeros, then drops
                // whole the code-blocks it had included)
                if drop && self.drop_new_passes(res, pi.precno) && !skip {
                    self.stats.cut_in_decoded = true;
                }
                match &mut self.headers {
                    PacketHeaders::Inline => {}
                    PacketHeaders::Ppm(data, cursor) => cursor.pos = data.len(),
                    PacketHeaders::Ppt(data, cursor) => cursor.pos = data.len(),
                }
                return Ok((false, p_max_length));
            }
        };
        match &mut self.headers {
            PacketHeaders::Inline => current += used,
            PacketHeaders::Ppm(_, cursor) => cursor.pos += used,
            PacketHeaders::Ppt(_, cursor) => cursor.pos += used,
        }
        Ok((present, current))
    }

    /// `opj_t2_read_packet_data`: the bytes read. `src` is the tile's data
    /// from the packet's body on, at `base` in the tile's data.
    fn read_packet_data(&mut self, res: &Resolution, pi: PacketPos, src_len: usize, base: usize) -> Result<usize, Error> {
        let mut current = 0usize;
        // the bytes left: segments are checked against it as lengths
        let mut left = src_len;
        let mut partial_buffer = false;
        for bandno in 0..res.numbands as usize {
            let Some(prc) = self.band_precinct(res, bandno, pi.precno) else {
                continue;
            };
            let n = (prc.cw * prc.ch) as usize;
            for cblk in &mut self.cblks[prc.cblks as usize..prc.cblks as usize + n] {
                if cblk.numnewpasses == 0 {
                    continue;
                }
                let mut seg = first_new_seg(cblk);
                if partial_buffer || cblk.corrupted {
                    // the packet's data ended before this contribution, or the code-block lost
                    // passes of an earlier packet: these passes are left out, everything earlier
                    // packets gave stays, and the passes count in the header state so packed
                    // headers stay in step
                    skip_cblk_passes(cblk, seg);
                    cblk.corrupted = true;
                    self.stats.cut_in_decoded = true;
                    continue;
                }
                loop {
                    let Some(s) = cblk.segs.get(seg).copied() else {
                        return Err(Error::Data("code-block segment state out of step"));
                    };
                    if s.newlen as usize > left {
                        // the data ended inside this contribution: keep the segments it
                        // completed before the cut and everything earlier packets gave; the
                        // code-block takes no more passes
                        partial_buffer = true;
                        cblk.real_num_segs = cblk.numsegs - u32::from(s.numpasses == 0);
                        skip_cblk_passes(cblk, seg);
                        cblk.corrupted = true;
                        self.stats.cut_in_decoded = true;
                        break;
                    }
                    cblk.chunks.push(Chunk {
                        pos: (base + current) as u32,
                        len: s.newlen,
                    });
                    current += s.newlen as usize;
                    left -= s.newlen as usize;
                    let s = &mut cblk.segs[seg];
                    s.len = s.len.wrapping_add(s.newlen);
                    s.numpasses += s.numnewpasses;
                    cblk.numnewpasses = cblk.numnewpasses.saturating_sub(s.numnewpasses);
                    s.real_num_passes = s.numpasses;
                    if cblk.numnewpasses == 0 {
                        break;
                    }
                    if s.numnewpasses == 0 {
                        return Err(Error::Data("code-block segment state out of step"));
                    }
                    seg += 1;
                    cblk.numsegs += 1;
                }
                if !partial_buffer {
                    cblk.real_num_segs = cblk.numsegs;
                }
            }
        }
        Ok(if partial_buffer { src_len } else { src_len - left })
    }

    /// `opj_t2_skip_packet_data`: the bytes read.
    fn skip_packet_data(&mut self, res: &Resolution, pi: PacketPos, max_length: usize) -> Result<usize, Error> {
        let mut cut = false;
        let mut data_read = 0usize;
        for bandno in 0..res.numbands as usize {
            let Some(prc) = self.band_precinct(res, bandno, pi.precno) else {
                continue;
            };
            let n = (prc.cw * prc.ch) as usize;
            for cblk in &mut self.cblks[prc.cblks as usize..prc.cblks as usize + n] {
                if cblk.numnewpasses == 0 {
                    continue;
                }
                let mut seg = first_new_seg(cblk);
                if cut {
                    skip_cblk_passes(cblk, seg);
                    continue;
                }
                loop {
                    let Some(s) = cblk.segs.get_mut(seg) else {
                        return Err(Error::Data("code-block segment state out of step"));
                    };
                    if data_read + s.newlen as usize > max_length {
                        // the data ended inside this contribution: the rest of the tile's data
                        // counts as read; the passes still count in the header state
                        cut = true;
                        skip_cblk_passes(cblk, seg);
                        break;
                    }
                    data_read += s.newlen as usize;
                    s.numpasses += s.numnewpasses;
                    cblk.numnewpasses = cblk.numnewpasses.saturating_sub(s.numnewpasses);
                    if cblk.numnewpasses == 0 {
                        break;
                    }
                    if s.numnewpasses == 0 {
                        return Err(Error::Data("code-block segment state out of step"));
                    }
                    seg += 1;
                    cblk.numsegs += 1;
                }
            }
        }
        Ok(if cut { max_length } else { data_read })
    }

    /// `opj_t2_decode_packet` / `opj_t2_skip_packet`: the bytes of the
    /// tile's data the packet took. `src` is the tile's data from the packet
    /// on, at `base` in the tile's data.
    fn visit_packet(&mut self, res: &Resolution, cblksty: u32, pi: PacketPos, src: &[u8], base: usize, skip: bool) -> Result<usize, Error> {
        let (read_data, header_bytes) = self.read_packet_header(res, cblksty, pi, src, skip)?;
        let mut total = header_bytes;
        if read_data {
            let left = src.len() - header_bytes;
            total += if skip {
                self.skip_packet_data(res, pi, left)?
            } else {
                self.read_packet_data(res, pi, left, base + header_bytes)?
            };
        }
        Ok(total)
    }
}

/// What a tile's packets need from the codestream.
pub(crate) struct PacketInput<'a, 'b> {
    pub mh: &'a MainHeader,
    pub tile: u32,
    /// The tile-components' parameters, precedence applied.
    pub params: &'a [TileComponentParams<'a>],
    /// The tile's bytes after SOD, every tile-part's in turn.
    pub data: &'a [u8],
    /// The bytes ended before the tile-parts' declared lengths.
    pub cut: bool,
    pub headers: PacketHeaders<'b>,
}

/// Tier-2: every packet in OpenJPEG's order, with the cut semantics of the
/// module's header and the exhaustion exit. `resno_decoded` receives each
/// component's decoded resolution (`opj_image_comp_t`), `included` is room
/// for the table of packets met.
pub(crate) fn read_packets(
    tile: &mut Tile,
    input: PacketInput<'_, '_>,
    resno_decoded: &mut [u32],
    included: &mut Vec<u64>,
) -> Result<PacketStats, Error> {
    let mh = input.mh;
    let coding = input.params[0].coding;
    let numcomps = mh.numcomps();
    let numlayers = u32::from(coding.numlayers);

    // the progression changes: the tile's own replace the main header's, as
    // ISO 15444-1 A.6.6 has it (OpenJPEG appends the tile's to the main
    // header's: departure DP-12)
    let pocs: &[PocEntry] = match mh.overrides(input.tile) {
        Some(ov) if !ov.poc.is_empty() => &ov.poc,
        _ => &mh.poc,
    };

    // opj_get_all_encoding_parameters: the tile's area, the most resolutions and precincts
    let (p, q) = (input.tile % mh.tw, input.tile / mh.tw);
    let l_tx0 = mh.tx0.wrapping_add(p.wrapping_mul(mh.tdx));
    let l_ty0 = mh.ty0.wrapping_add(q.wrapping_mul(mh.tdy));
    let mut layout = PiLayout {
        comps: Vec::with_capacity(numcomps as usize),
        resolutions: Vec::new(),
        tx0: l_tx0.max(mh.x0),
        ty0: l_ty0.max(mh.y0),
        tx1: uint_adds(l_tx0, mh.tdx).min(mh.x1),
        ty1: uint_adds(l_ty0, mh.tdy).min(mh.y1),
        step_l: 0,
        step_r: 0,
        step_c: 0,
    };
    let (mut max_prec, mut max_res) = (0u32, 0u32);
    for (sc, params) in mh.comps.iter().zip(input.params) {
        let style = params.style;
        let numres = u32::from(style.numresolutions);
        let (dx, dy) = (u32::from(sc.dx), u32::from(sc.dy));
        let (tcx0, tcy0) = (ceildiv(layout.tx0, dx), ceildiv(layout.ty0, dy));
        let (tcx1, tcy1) = (ceildiv(layout.tx1, dx), ceildiv(layout.ty1, dy));
        max_res = max_res.max(numres);
        layout.comps.push(PiComp {
            dx,
            dy,
            first: layout.resolutions.len(),
            numresolutions: numres,
        });
        let mut level_no = numres;
        for resno in 0..numres {
            level_no -= 1;
            let pdx = u32::from(style.precincts[resno as usize] & 0xF);
            let pdy = u32::from(style.precincts[resno as usize] >> 4);
            let (rx0, ry0) = (uint_ceildivpow2(tcx0, level_no), uint_ceildivpow2(tcy0, level_no));
            let (rx1, ry1) = (uint_ceildivpow2(tcx1, level_no), uint_ceildivpow2(tcy1, level_no));
            let (px0, py0) = ((rx0 >> pdx) << pdx, (ry0 >> pdy) << pdy);
            let (px1, py1) = (
                uint_ceildivpow2(rx1, pdx).wrapping_shl(pdx),
                uint_ceildivpow2(ry1, pdy).wrapping_shl(pdy),
            );
            let pw = if rx0 == rx1 { 0 } else { px1.wrapping_sub(px0) >> pdx };
            let ph = if ry0 == ry1 { 0 } else { py1.wrapping_sub(py0) >> pdy };
            layout.resolutions.push(PiResolution { pdx, pdy, pw, ph });
            max_prec = max_prec.max(pw.wrapping_mul(ph));
        }
    }
    // the table of packets met: (layers + 1) x resolutions x components x precincts bits
    let step_c = max_prec;
    let step_r = numcomps.wrapping_mul(step_c);
    let step_l = max_res.wrapping_mul(step_r);
    if step_l > u32::MAX / (numlayers + 1) {
        return Err(Error::Limit("the table of packets exceeds system limits"));
    }
    layout.step_c = u64::from(step_c);
    layout.step_r = u64::from(step_r);
    layout.step_l = u64::from(step_l);
    let size = u64::from(numlayers + 1) * u64::from(step_l);
    included.clear();
    included.resize(size.div_ceil(64) as usize + 1, 0);
    let mut included = Included { bits: included, size };

    let Tile {
        comps,
        res,
        precincts,
        cblks,
        tag_nodes,
        ..
    } = tile;
    let comps = &comps[..];
    let mut t2 = T2 {
        res,
        precincts,
        cblks,
        tag_nodes,
        csty: coding.csty,
        cut: input.cut,
        headers: input.headers,
        cut_sop: false,
        stats: PacketStats::default(),
    };

    let data = input.data;
    let mut at = 0usize;
    let mut exhausted = false;
    let mut nb_comps_final = 0u32;
    resno_decoded.fill(0);
    let mut first_pass_failed = vec![true; numcomps as usize];

    let bound = pocs.len().max(1);
    for pino in 0..bound {
        let poc = match pocs.get(pino) {
            // opj_pi_update_decode_poc (the layer end bounded by the tile's
            // layers, the component end by the components)
            Some(e) => Progression {
                prg: e.prg,
                resno0: u32::from(e.resno0),
                compno0: u32::from(e.compno0),
                resno1: u32::from(e.resno1),
                compno1: u32::from(e.compno1).min(numcomps),
                layno1: u32::from(e.layno1).min(numlayers),
            },
            // opj_pi_update_decode_not_poc
            None => Progression {
                prg: coding.prg,
                resno0: 0,
                compno0: 0,
                resno1: max_res,
                compno1: numcomps,
                layno1: numlayers,
            },
        };
        first_pass_failed.fill(true);

        let flow = for_each_packet(&layout, &mut included, &poc, |pi| {
            let c = pi.compno as usize;
            let tilec = &comps[c];
            let res = t2.res[(tilec.res + pi.resno) as usize];
            if pi.precno >= res.pw * res.ph {
                // (a position-driven order's precinct outside the resolution's:
                // OpenJPEG's "Invalid precinct")
                return ControlFlow::Break(Err(Error::Data("Invalid precinct")));
            }
            let skip_packet = if pi.resno >= tilec.minimum_num_resolutions {
                true
            } else {
                // a precinct that meets the area of interest in some band
                // (the components not decoded included: OpenJPEG's Tier-2
                // reads their packets as the others)
                !res.bands[..res.numbands as usize].iter().any(|band| {
                    let prec = t2.precincts[(band.precincts + pi.precno) as usize];
                    area_of_interest(
                        tilec,
                        pi.resno,
                        band.bandno,
                        prec.x0 as u32,
                        prec.y0 as u32,
                        prec.x1 as u32,
                        prec.y1 as u32,
                    )
                })
            };
            let last_resno = tilec.minimum_num_resolutions - 1;
            if exhausted {
                // an absent packet: its visit would change only its component's decoded resolution
                if resno_decoded[c] < last_resno {
                    if !skip_packet {
                        first_pass_failed[c] = false;
                        resno_decoded[c] = resno_decoded[c].max(pi.resno);
                    } else if first_pass_failed[c] && resno_decoded[c] == 0 {
                        resno_decoded[c] = last_resno;
                    }
                    if resno_decoded[c] >= last_resno {
                        nb_comps_final += 1;
                        if nb_comps_final == numcomps {
                            return ControlFlow::Break(Ok(()));
                        }
                    }
                }
                return ControlFlow::Continue(());
            }
            if !skip_packet {
                first_pass_failed[c] = false;
            }
            let read = match t2.visit_packet(&res, u32::from(tilec.cblksty), pi, &data[at..], at, skip_packet) {
                Ok(n) => n,
                Err(e) => return ControlFlow::Break(Err(if t2.cut_sop { Error::NeedData("a packet's SOP marker") } else { e })),
            };
            if !skip_packet {
                resno_decoded[c] = resno_decoded[c].max(pi.resno);
            }
            if first_pass_failed[c] && resno_decoded[c] == 0 {
                resno_decoded[c] = last_resno;
            }
            at += read;
            let headers_left = match &t2.headers {
                PacketHeaders::Inline => false,
                PacketHeaders::Ppm(d, cursor) => cursor.pos < d.len(),
                PacketHeaders::Ppt(d, cursor) => cursor.pos < d.len(),
            };
            if at == data.len() && !headers_left {
                // the tile's data is exhausted: every remaining packet would
                // read an empty header and be absent
                exhausted = true;
                nb_comps_final = 0;
                for (r, tc) in resno_decoded.iter().zip(comps) {
                    if r + 1 >= tc.minimum_num_resolutions {
                        nb_comps_final += 1;
                    }
                }
                if nb_comps_final == numcomps {
                    return ControlFlow::Break(Ok(()));
                }
            }
            ControlFlow::Continue(())
        });
        if let ControlFlow::Break(result) = flow {
            result?;
            break;
        }
    }
    Ok(t2.stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bio_reads_bits_and_stuffs_after_ff() {
        // 0xFF is followed by a byte of 7 bits
        let mut bio = Bio::new(&[0b1010_0000, 0xFF, 0b0100_0000]);
        assert_eq!(bio.read(3), 0b101);
        assert_eq!(bio.read(5), 0);
        assert_eq!(bio.read(8), 0xFF);
        assert_eq!(bio.read(7), 0b100_0000);
        assert!(!bio.overrun);
        assert_eq!(bio.read(1), 0);
        assert!(bio.overrun);
    }

    #[test]
    fn numpasses_code() {
        assert_eq!(getnumpasses(&mut Bio::new(&[0b0000_0000])), 1);
        assert_eq!(getnumpasses(&mut Bio::new(&[0b1000_0000])), 2);
        assert_eq!(getnumpasses(&mut Bio::new(&[0b1101_0000])), 4);
        assert_eq!(getnumpasses(&mut Bio::new(&[0b1111_0001, 0b0000_0000])), 8);
    }

    #[test]
    fn tag_tree_decodes_a_leaf() {
        // a single-node tree whose value is 2: bits 0, 0, 1
        let mut nodes = vec![TagNode { value: 999, low: 0 }];
        let t = TagTree { off: 0, w: 1, h: 1 };
        let mut bio = Bio::new(&[0b0010_0000]);
        assert!(!tgt_decode(&mut bio, &mut nodes, &t, 0, 1));
        assert!(!tgt_decode(&mut bio, &mut nodes, &t, 0, 2));
        assert!(tgt_decode(&mut bio, &mut nodes, &t, 0, 3));
        assert_eq!(nodes[0].value, 2);
    }

    #[test]
    fn positions_step_to_multiples() {
        assert_eq!(positions(3, 20, 8).collect::<Vec<_>>(), vec![3, 8, 16]);
        assert_eq!(positions(0, 16, 8).collect::<Vec<_>>(), vec![0, 8]);
        assert_eq!(positions(5, 5, 8).count(), 0);
    }
}
