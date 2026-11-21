// This file is a port and keeps the notices of what it ports (both under the
// BSD 2-Clause License, whose text NOTICE.md gives, "JPEG 2000 decoder").
//
// neo-j2k, `src/t1.cpp`:
//   Copyright (c) 2026, Anne Skydancer (neo-j2k)
// OpenJPEG, through it:
//   Copyright (c) 2002-2014, Universite catholique de Louvain (UCL), Belgium
//   Copyright (c) 2002-2014, Professor Benoit Macq
//   Copyright (c) 2001-2003, David Janssens
//   Copyright (c) 2002-2003, Yannick Verschueren
//   Copyright (c) 2003-2007, Francois-Olivier Devaux
//   Copyright (c) 2003-2014, Antonin Descampe
//   Copyright (c) 2005, Herve Drolon, FreeImage Team
//   Copyright (c) 2007, Callum Lerwick <seg@haxxed.com>
//   Copyright (c) 2008, Jerome Fimes, Communications & Systemes <jerome.fimes@c-s.fr>
//   Copyright (c) 2012, Carl Hetherington
//   Copyright (c) 2017, IntoPIX SA <support@intopix.com>
//   All rights reserved.

//! Tier-1 decoding of one code-block: the MQ and raw (BYPASS) decoders and
//! the three coding passes.
//!
//! Port of neo-j2k's `src/t1.cpp` (BSD-2-Clause, Copyright (c) 2026 Anne
//! Skydancer), itself ported from OpenJPEG 2.5.4 (`t1.c`, `mqc.c`,
//! `mqc_inl.h`, `t1_luts.h`; BSD-2-Clause, see NOTICE.md) with the same
//! operations, so the coefficients are OpenJPEG's. From neo-j2k: the MQ
//! renormalisation counts its shifts at once (count-leading-zeros) instead
//! of one bit at a time, with the same byte reads at the same shifts.
//!
//! The code-block's bytes are always copied into the decoder's padded
//! buffer first, so the MQ decoder's 0xFFFF sentinel is never written into
//! the caller's bytes.
//!
//! No `unsafe`: the coefficient and flag buffers have 4096 entries, a power
//! of two, and every index into them is masked, which costs an AND where the
//! C code has nothing and a bounds check would cost a branch. A code-block
//! has at most 4096 samples (the header refuses more) and its flags at most
//! 3078 entries, so the mask never changes an index of a valid block.

use crate::t1_luts::{LUT_CTXNO_SC, LUT_CTXNO_ZC, LUT_SPB, MQC_WORDS};
use crate::tile::{Chunk, Segment};

const CBLK_LAZY: u32 = 0x01;
const CBLK_RESET: u32 = 0x02;
const CBLK_VSC: u32 = 0x08;
const CBLK_SEGSYM: u32 = 0x20;

// OpenJPEG's t1.h: the contexts and the flags of `opj_flag_t`
const T1_CTXNO_ZC: usize = 0;
const T1_CTXNO_MAG: usize = 14;
const T1_CTXNO_AGG: usize = 17;
const T1_CTXNO_UNI: usize = 18;
const T1_SIGMA_0: u32 = 1 << 0;
const T1_SIGMA_1: u32 = 1 << 1;
const T1_SIGMA_2: u32 = 1 << 2;
const T1_SIGMA_3: u32 = 1 << 3;
const T1_SIGMA_4: u32 = 1 << 4;
const T1_SIGMA_5: u32 = 1 << 5;
const T1_SIGMA_7: u32 = 1 << 7;
const T1_SIGMA_15: u32 = 1 << 15;
const T1_SIGMA_16: u32 = 1 << 16;
const T1_SIGMA_17: u32 = 1 << 17;
const T1_CHI_0_I: u32 = 18;
const T1_CHI_1_I: u32 = 19;
const T1_MU_0: u32 = 1 << 20;
const T1_PI_0: u32 = 1 << 21;
const T1_CHI_2_I: u32 = 22;
const T1_PI_1: u32 = 1 << 24;
const T1_PI_2: u32 = 1 << 27;
const T1_PI_3: u32 = 1 << 30;
const T1_CHI_5_I: u32 = 31;
/// NW, N, NE, W, E, SW, S, SE.
const T1_SIGMA_NEIGHBOURS: u32 = 0x1EF;
const T1_SIGMA_THIS: u32 = T1_SIGMA_4;
const T1_CHI_THIS_I: u32 = T1_CHI_1_I;
const T1_MU_THIS: u32 = T1_MU_0;
const T1_PI_THIS: u32 = T1_PI_0;
const T1_PI_ALL: u32 = T1_PI_0 | T1_PI_1 | T1_PI_2 | T1_PI_3;

/// Samples of the largest code-block, and the size of the two buffers.
const BUF: usize = 4096;
const MASK: usize = BUF - 1;
/// `OPJ_COMMON_CBLK_DATA_EXTRA`: the sentinel's bytes after a segment.
const EXTRA: usize = 2;

/// What Tier-1 needs of a code-block.
pub(crate) struct T1Block<'a> {
    /// `x1 - x0`, `y1 - y0` (at most 1024 each, 4096 samples).
    pub w: u32,
    pub h: u32,
    /// From its packet headers (0 when never included).
    pub numbps: u32,
    /// The band: 0 LL, 1 HL, 2 LH, 3 HH.
    pub orient: u32,
    pub roishift: u32,
    pub cblksty: u32,
    pub chunks: &'a [Chunk],
    /// The segments with data.
    pub segs: &'a [Segment],
}

/// The MQ decoder's registers (`opj_mqc_t`, decoding side) and its place in
/// the code-block's bytes. The passes work on a copy that stays in machine
/// registers.
#[derive(Clone, Copy, Default)]
struct Mq {
    a: u32,
    c: u32,
    ct: u32,
    bp: usize,
}

/// The 19 contexts, each holding its state's word of `MQC_WORDS` (the
/// state's probability estimate, its MPS and its two next states), padded
/// to 32: a decision reads its context once, and the table only when the
/// state changes.
type Contexts = [u32; 32];

/// `opj_mqc_bytein_macro`. The sentinel (0xFF 0xFF after the segment) stops
/// `bp` at the segment's end, so `bp + 1` stays inside the buffer.
#[inline(always)]
fn mq_bytein(mq: &mut Mq, buf: &[u8]) {
    let l_c = u32::from(buf[mq.bp + 1]);
    if buf[mq.bp] == 0xff {
        if l_c > 0x8f {
            mq.c = mq.c.wrapping_add(0xff00);
            mq.ct = 8;
        } else {
            mq.bp += 1;
            mq.c = mq.c.wrapping_add(l_c << 9);
            mq.ct = 7;
        }
    } else {
        mq.bp += 1;
        mq.c = mq.c.wrapping_add(l_c << 8);
        mq.ct = 8;
    }
}

/// `opj_mqc_renormd_macro`, as neo-j2k has it: the shifts counted at once
/// and taken as many at a time as `c` has bits left, a byte read whenever
/// none is left: the same byte reads at the same shifts as OpenJPEG's
/// one-bit loop (`a` is 1 to 0x7fff here: 1 to 15 shifts).
#[inline(always)]
fn mq_renorm(mq: &mut Mq, buf: &[u8]) {
    let mut shifts = (mq.a.leading_zeros().wrapping_sub(16)) & 31;
    loop {
        if mq.ct == 0 {
            mq_bytein(mq, buf);
        }
        let k = shifts.min(mq.ct);
        mq.a <<= k;
        mq.c <<= k;
        mq.ct -= k;
        shifts -= k;
        if shifts == 0 {
            break;
        }
    }
}

/// `opj_mqc_decode_macro`: ISO 15444-1 C.3.2, decoding a decision.
#[inline(always)]
fn mq_decode(mq: &mut Mq, ctxs: &mut Contexts, buf: &[u8], cx: usize) -> u32 {
    let cx = cx & 31;
    let w = ctxs[cx];
    let qeval = w & 0xffff;
    let mps = (w >> 16) & 1;
    mq.a = mq.a.wrapping_sub(qeval);
    if (mq.c >> 16) < qeval {
        // opj_mqc_lpsexchange_macro
        let lps = mq.a >= qeval;
        let next = if lps { w >> 24 } else { (w >> 17) & 0x7f };
        ctxs[cx] = MQC_WORDS[(next & 127) as usize];
        mq.a = qeval;
        mq_renorm(mq, buf);
        mps ^ u32::from(lps)
    } else {
        mq.c = mq.c.wrapping_sub(qeval << 16);
        if mq.a & 0x8000 == 0 {
            // opj_mqc_mpsexchange_macro
            let lps = mq.a < qeval;
            let next = if lps { w >> 24 } else { (w >> 17) & 0x7f };
            ctxs[cx] = MQC_WORDS[(next & 127) as usize];
            mq_renorm(mq, buf);
            mps ^ u32::from(lps)
        } else {
            mps
        }
    }
}

/// `opj_mqc_raw_decode`: a bit of a raw (BYPASS) segment. Here `c` is the
/// current byte.
#[inline(always)]
fn raw_decode(mq: &mut Mq, buf: &[u8]) -> u32 {
    if mq.ct == 0 {
        // the sentinel after the segment stops the reads
        let b = u32::from(buf[mq.bp]);
        if mq.c == 0xff {
            if b > 0x8f {
                mq.c = 0xff;
                mq.ct = 8;
            } else {
                mq.c = b;
                mq.bp += 1;
                mq.ct = 7;
            }
        } else {
            mq.c = b;
            mq.bp += 1;
            mq.ct = 8;
        }
    }
    mq.ct -= 1;
    (mq.c >> mq.ct) & 1
}

/// `opj_mqc_resetstates` and the three `opj_mqc_setstate` that follow it
/// everywhere: UNI at state 46, AGG at 3, the first ZC context at 4.
fn reset_contexts(ctxs: &mut Contexts) {
    *ctxs = [MQC_WORDS[0]; 32];
    ctxs[T1_CTXNO_UNI] = MQC_WORDS[46 << 1];
    ctxs[T1_CTXNO_AGG] = MQC_WORDS[3 << 1];
    ctxs[T1_CTXNO_ZC] = MQC_WORDS[4 << 1];
}

/// `opj_t1_getctxtno_sc_or_spb_index`, for the row `CI` of a stripe.
#[inline(always)]
fn sc_or_spb_index<const CI: u32>(fx: u32, pfx: u32, nfx: u32) -> usize {
    let mut lu = (fx >> (CI * 3)) & (T1_SIGMA_1 | T1_SIGMA_3 | T1_SIGMA_5 | T1_SIGMA_7);
    lu |= (pfx >> (T1_CHI_THIS_I + CI * 3)) & (1 << 0);
    lu |= (nfx >> (T1_CHI_THIS_I - 2 + CI * 3)) & (1 << 2);
    if CI == 0 {
        lu |= (fx >> (T1_CHI_0_I - 4)) & (1 << 4);
    } else {
        lu |= (fx >> (T1_CHI_1_I - 4 + CI.saturating_sub(1) * 3)) & (1 << 4);
    }
    lu |= (fx >> (T1_CHI_2_I - 6 + CI * 3)) & (1 << 6);
    (lu & 255) as usize
}

/// `opj_t1_getctxno_mag`
#[inline(always)]
fn ctxno_mag(f: u32) -> usize {
    if f & T1_MU_0 != 0 {
        T1_CTXNO_MAG + 2
    } else if f & T1_SIGMA_NEIGHBOURS != 0 {
        T1_CTXNO_MAG + 1
    } else {
        T1_CTXNO_MAG
    }
}

/// A code-block's coefficients, flags and contexts, in one piece: the
/// passes then address all three from one register.
struct Work {
    /// `(h + 3) / 4 + 2` rows of `w + 2`.
    flags: [u32; BUF],
    /// `w` x `h` coefficients of the current code-block, row-major.
    data: [i32; BUF],
    ctxs: Contexts,
}

/// What the passes of a code-block work on.
struct Coder<'a> {
    work: &'a mut Work,
    /// The code-block's bytes, the current segment ended by the sentinel.
    buf: &'a [u8],
    /// `orient << 9`: the band's part of `LUT_CTXNO_ZC`.
    zc: usize,
}

/// A step of a pass on the row `$ci` (0 to 3, known only when the pass
/// runs) of a stripe: the step compiled for that row.
macro_rules! step {
    ($ci:expr, $self:ident . $f:ident ( $($a:expr),* )) => {
        match $ci {
            0 => $self.$f::<0>($($a),*),
            1 => $self.$f::<1>($($a),*),
            2 => $self.$f::<2>($($a),*),
            _ => $self.$f::<3>($($a),*),
        }
    };
}

// Each step is compiled once for each of the four rows of a stripe (`CI`),
// as OpenJPEG's macros expand: the row's shifts are constants.
impl Coder<'_> {
    /// `opj_t1_getctxno_zc`
    #[inline(always)]
    fn ctxno_zc(&self, f: u32) -> usize {
        usize::from(LUT_CTXNO_ZC[(self.zc | (f & T1_SIGMA_NEIGHBOURS) as usize) & 2047])
    }

    /// `opj_t1_update_flags_macro`: the sample at `fp`, row `CI` of its
    /// stripe, becomes significant with sign `s`; `flags` is the column's
    /// own entry, held by the caller.
    #[inline(always)]
    fn update_flags<const CI: u32>(&mut self, flags: &mut u32, fp: usize, s: u32, stride: usize, vsc: bool) {
        // east
        self.work.flags[(fp - 1) & MASK] |= T1_SIGMA_5 << (3 * CI);
        // mark target as significant
        *flags |= ((s << T1_CHI_1_I) | T1_SIGMA_4) << (3 * CI);
        // west
        self.work.flags[(fp + 1) & MASK] |= T1_SIGMA_3 << (3 * CI);
        // north-west, north, north-east
        if CI == 0 && !vsc {
            let north = fp - stride;
            self.work.flags[north & MASK] |= (s << T1_CHI_5_I) | T1_SIGMA_16;
            self.work.flags[(north - 1) & MASK] |= T1_SIGMA_17;
            self.work.flags[(north + 1) & MASK] |= T1_SIGMA_15;
        }
        // south-west, south, south-east
        if CI == 3 {
            let south = fp + stride;
            self.work.flags[south & MASK] |= (s << T1_CHI_0_I) | T1_SIGMA_1;
            self.work.flags[(south - 1) & MASK] |= T1_SIGMA_2;
            self.work.flags[(south + 1) & MASK] |= T1_SIGMA_0;
        }
    }

    /// The sign of a sample that became significant, and its flags
    /// (`opj_t1_dec_sigpass_step_mqc_macro`'s and
    /// `opj_t1_dec_clnpass_step_macro`'s second half).
    #[inline(always)]
    fn sign<const CI: u32>(&mut self, mq: &mut Mq, flags: &mut u32, fp: usize, stride: usize, di: usize, oneplushalf: i32, vsc: bool) {
        let lu = sc_or_spb_index::<CI>(*flags, self.work.flags[(fp - 1) & MASK], self.work.flags[(fp + 1) & MASK]);
        let v = mq_decode(mq, &mut self.work.ctxs, self.buf, usize::from(LUT_CTXNO_SC[lu])) ^ u32::from(LUT_SPB[lu]);
        self.work.data[di & MASK] = if v != 0 { -oneplushalf } else { oneplushalf };
        self.update_flags::<CI>(flags, fp, v, stride, vsc);
    }

    /// `opj_t1_dec_sigpass_step_mqc_macro`
    #[inline(always)]
    fn sig_step_mqc<const CI: u32>(
        &mut self,
        mq: &mut Mq,
        flags: &mut u32,
        fp: usize,
        stride: usize,
        di: usize,
        oneplushalf: i32,
        vsc: bool,
    ) {
        if (*flags & ((T1_SIGMA_THIS | T1_PI_THIS) << (CI * 3))) == 0 && (*flags & (T1_SIGMA_NEIGHBOURS << (CI * 3))) != 0 {
            let ctxt1 = self.ctxno_zc(*flags >> (CI * 3));
            if mq_decode(mq, &mut self.work.ctxs, self.buf, ctxt1) != 0 {
                self.sign::<CI>(mq, flags, fp, stride, di, oneplushalf, vsc);
            }
            *flags |= T1_PI_THIS << (CI * 3);
        }
    }

    /// `opj_t1_dec_sigpass_step_raw`
    #[inline(always)]
    fn sig_step_raw<const CI: u32>(
        &mut self,
        mq: &mut Mq,
        flags: &mut u32,
        fp: usize,
        stride: usize,
        di: usize,
        oneplushalf: i32,
        vsc: bool,
    ) {
        if (*flags & ((T1_SIGMA_THIS | T1_PI_THIS) << (CI * 3))) == 0 && (*flags & (T1_SIGMA_NEIGHBOURS << (CI * 3))) != 0 {
            if raw_decode(mq, self.buf) != 0 {
                let v = raw_decode(mq, self.buf);
                self.work.data[di & MASK] = if v != 0 { -oneplushalf } else { oneplushalf };
                self.update_flags::<CI>(flags, fp, v, stride, vsc);
            }
            *flags |= T1_PI_THIS << (CI * 3);
        }
    }

    /// `opj_t1_dec_refpass_step_mqc_macro` and `opj_t1_dec_refpass_step_raw`
    #[inline(always)]
    fn ref_step<const CI: u32>(&mut self, mq: &mut Mq, flags: &mut u32, di: usize, poshalf: i32, raw: bool) {
        if (*flags & ((T1_SIGMA_THIS | T1_PI_THIS) << (CI * 3))) == (T1_SIGMA_THIS << (CI * 3)) {
            let v = if raw {
                raw_decode(mq, self.buf)
            } else {
                mq_decode(mq, &mut self.work.ctxs, self.buf, ctxno_mag(*flags >> (CI * 3)))
            };
            let d = &mut self.work.data[di & MASK];
            *d = d.wrapping_add(if (v ^ u32::from(*d < 0)) != 0 { poshalf } else { -poshalf });
            *flags |= T1_MU_THIS << (CI * 3);
        }
    }

    /// `opj_t1_dec_clnpass_step_macro`
    #[inline(always)]
    fn cln_step<const CI: u32>(
        &mut self,
        mq: &mut Mq,
        check_flags: bool,
        partial: bool,
        flags: &mut u32,
        fp: usize,
        stride: usize,
        di: usize,
        oneplushalf: i32,
        vsc: bool,
    ) {
        if !check_flags || (*flags & ((T1_SIGMA_THIS | T1_PI_THIS) << (CI * 3))) == 0 {
            if !partial {
                let ctxt1 = self.ctxno_zc(*flags >> (CI * 3));
                if mq_decode(mq, &mut self.work.ctxs, self.buf, ctxt1) == 0 {
                    return;
                }
            }
            self.sign::<CI>(mq, flags, fp, stride, di, oneplushalf, vsc);
        }
    }

    /// `opj_t1_dec_sigpass_mqc_internal` and `opj_t1_dec_sigpass_raw`: the
    /// significance propagation pass.
    #[inline(always)]
    fn sigpass(&mut self, mq_io: &mut Mq, bpno: u32, vsc: bool, w: usize, h: usize, stride: usize, raw: bool) {
        let mut mq = *mq_io;
        let one = 1i32 << bpno;
        let oneplushalf = one | (one >> 1);
        let mut dp = 0usize;
        let mut fp = stride + 1;
        let mut k = 0;
        while k < (h & !3) {
            for _ in 0..w {
                let mut flags = self.work.flags[fp & MASK];
                if flags != 0 {
                    if raw {
                        self.sig_step_raw::<0>(&mut mq, &mut flags, fp, stride, dp, oneplushalf, vsc);
                        self.sig_step_raw::<1>(&mut mq, &mut flags, fp, stride, dp + w, oneplushalf, vsc);
                        self.sig_step_raw::<2>(&mut mq, &mut flags, fp, stride, dp + 2 * w, oneplushalf, vsc);
                        self.sig_step_raw::<3>(&mut mq, &mut flags, fp, stride, dp + 3 * w, oneplushalf, vsc);
                    } else {
                        self.sig_step_mqc::<0>(&mut mq, &mut flags, fp, stride, dp, oneplushalf, vsc);
                        self.sig_step_mqc::<1>(&mut mq, &mut flags, fp, stride, dp + w, oneplushalf, vsc);
                        self.sig_step_mqc::<2>(&mut mq, &mut flags, fp, stride, dp + 2 * w, oneplushalf, vsc);
                        self.sig_step_mqc::<3>(&mut mq, &mut flags, fp, stride, dp + 3 * w, oneplushalf, vsc);
                    }
                    self.work.flags[fp & MASK] = flags;
                }
                dp += 1;
                fp += 1;
            }
            k += 4;
            dp += 3 * w;
            fp += 2;
        }
        if k < h {
            for _ in 0..w {
                let mut flags = self.work.flags[fp & MASK];
                for j in 0..h - k {
                    if raw {
                        step!(j, self.sig_step_raw(&mut mq, &mut flags, fp, stride, dp + j * w, oneplushalf, vsc));
                    } else {
                        step!(j, self.sig_step_mqc(&mut mq, &mut flags, fp, stride, dp + j * w, oneplushalf, vsc));
                    }
                }
                self.work.flags[fp & MASK] = flags;
                dp += 1;
                fp += 1;
            }
        }
        *mq_io = mq;
    }

    /// `opj_t1_dec_refpass_mqc_internal` and `opj_t1_dec_refpass_raw`: the
    /// magnitude refinement pass.
    #[inline(always)]
    fn refpass(&mut self, mq_io: &mut Mq, bpno: u32, w: usize, h: usize, stride: usize, raw: bool) {
        let mut mq = *mq_io;
        let one = 1i32 << bpno;
        let poshalf = one >> 1;
        let mut dp = 0usize;
        let mut fp = stride + 1;
        let mut k = 0;
        while k < (h & !3) {
            for _ in 0..w {
                let mut flags = self.work.flags[fp & MASK];
                if flags != 0 {
                    self.ref_step::<0>(&mut mq, &mut flags, dp, poshalf, raw);
                    self.ref_step::<1>(&mut mq, &mut flags, dp + w, poshalf, raw);
                    self.ref_step::<2>(&mut mq, &mut flags, dp + 2 * w, poshalf, raw);
                    self.ref_step::<3>(&mut mq, &mut flags, dp + 3 * w, poshalf, raw);
                    self.work.flags[fp & MASK] = flags;
                }
                dp += 1;
                fp += 1;
            }
            k += 4;
            dp += 3 * w;
            fp += 2;
        }
        if k < h {
            for _ in 0..w {
                let mut flags = self.work.flags[fp & MASK];
                for j in 0..h - k {
                    step!(j, self.ref_step(&mut mq, &mut flags, dp + j * w, poshalf, raw));
                }
                self.work.flags[fp & MASK] = flags;
                dp += 1;
                fp += 1;
            }
        }
        *mq_io = mq;
    }

    /// `opj_t1_dec_clnpass_internal`: the cleanup pass.
    #[inline(always)]
    fn clnpass(&mut self, mq_io: &mut Mq, bpno: u32, vsc: bool, w: usize, h: usize, stride: usize) {
        let mut mq = *mq_io;
        let one = 1i32 << bpno;
        let oneplushalf = one | (one >> 1);
        let mut dp = 0usize;
        let mut fp = stride + 1;
        let mut k = 0;
        while k < (h & !3) {
            for _ in 0..w {
                let mut flags = self.work.flags[fp & MASK];
                if flags == 0 {
                    // a column with nothing significant around it: the run-length mode
                    if mq_decode(&mut mq, &mut self.work.ctxs, self.buf, T1_CTXNO_AGG) != 0 {
                        let hi = mq_decode(&mut mq, &mut self.work.ctxs, self.buf, T1_CTXNO_UNI);
                        let lo = mq_decode(&mut mq, &mut self.work.ctxs, self.buf, T1_CTXNO_UNI);
                        // the first significant row of the column, known
                        // without its decision; the rows after it as usual
                        let runlen = (hi << 1) | lo;
                        if runlen == 0 {
                            self.cln_step::<0>(&mut mq, false, true, &mut flags, fp, stride, dp, oneplushalf, vsc);
                        }
                        if runlen <= 1 {
                            self.cln_step::<1>(&mut mq, false, runlen == 1, &mut flags, fp, stride, dp + w, oneplushalf, vsc);
                        }
                        if runlen <= 2 {
                            self.cln_step::<2>(&mut mq, false, runlen == 2, &mut flags, fp, stride, dp + 2 * w, oneplushalf, vsc);
                        }
                        self.cln_step::<3>(&mut mq, false, runlen == 3, &mut flags, fp, stride, dp + 3 * w, oneplushalf, vsc);
                        self.work.flags[fp & MASK] = flags & !T1_PI_ALL;
                    }
                } else {
                    self.cln_step::<0>(&mut mq, true, false, &mut flags, fp, stride, dp, oneplushalf, vsc);
                    self.cln_step::<1>(&mut mq, true, false, &mut flags, fp, stride, dp + w, oneplushalf, vsc);
                    self.cln_step::<2>(&mut mq, true, false, &mut flags, fp, stride, dp + 2 * w, oneplushalf, vsc);
                    self.cln_step::<3>(&mut mq, true, false, &mut flags, fp, stride, dp + 3 * w, oneplushalf, vsc);
                    self.work.flags[fp & MASK] = flags & !T1_PI_ALL;
                }
                dp += 1;
                fp += 1;
            }
            k += 4;
            dp += 3 * w;
            fp += 2;
        }
        if k < h {
            for _ in 0..w {
                let mut flags = self.work.flags[fp & MASK];
                for j in 0..h - k {
                    step!(
                        j,
                        self.cln_step(&mut mq, true, false, &mut flags, fp, stride, dp + j * w, oneplushalf, vsc)
                    );
                }
                self.work.flags[fp & MASK] = flags & !T1_PI_ALL;
                dp += 1;
                fp += 1;
            }
        }
        *mq_io = mq;
    }

    /// `opj_t1_dec_clnpass_check_segsym`: the segmentation symbol's four
    /// decisions, read and not checked, as in OpenJPEG.
    fn segsym(&mut self, mq: &mut Mq) {
        for _ in 0..4 {
            mq_decode(mq, &mut self.work.ctxs, self.buf, T1_CTXNO_UNI);
        }
    }
}

// The passes for the common 64 x 64 code-block and for any other size, as
// OpenJPEG has them: the first with its sizes known to the compiler.
#[inline(never)]
fn sigpass_64(cd: &mut Coder<'_>, mq: &mut Mq, bpno: u32, vsc: bool) {
    cd.sigpass(mq, bpno, vsc, 64, 64, 66, false);
}

#[inline(never)]
fn sigpass_generic(cd: &mut Coder<'_>, mq: &mut Mq, bpno: u32, vsc: bool, w: usize, h: usize, raw: bool) {
    cd.sigpass(mq, bpno, vsc, w, h, w + 2, raw);
}

#[inline(never)]
fn refpass_64(cd: &mut Coder<'_>, mq: &mut Mq, bpno: u32) {
    cd.refpass(mq, bpno, 64, 64, 66, false);
}

#[inline(never)]
fn refpass_generic(cd: &mut Coder<'_>, mq: &mut Mq, bpno: u32, w: usize, h: usize, raw: bool) {
    cd.refpass(mq, bpno, w, h, w + 2, raw);
}

#[inline(never)]
fn clnpass_64(cd: &mut Coder<'_>, mq: &mut Mq, bpno: u32, vsc: bool) {
    cd.clnpass(mq, bpno, vsc, 64, 64, 66);
}

#[inline(never)]
fn clnpass_generic(cd: &mut Coder<'_>, mq: &mut Mq, bpno: u32, vsc: bool, w: usize, h: usize) {
    cd.clnpass(mq, bpno, vsc, w, h, w + 2);
}

/// A decoder's Tier-1 scratch, kept between code-blocks and decodes.
pub(crate) struct T1 {
    work: Box<Work>,
    /// The code-block's bytes and 2 bytes of padding.
    cblkdata: Vec<u8>,
    w: usize,
    h: usize,
}

impl Default for T1 {
    fn default() -> Self {
        T1 {
            work: Box::new(Work {
                flags: [0; BUF],
                data: [0; BUF],
                ctxs: [0; 32],
            }),
            cblkdata: Vec::new(),
            w: 0,
            h: 0,
        }
    }
}

/// The bit-plane check of `opj_t1_decode_cblk` alone, for a code-block
/// without data (whose decode gives zeros): false when OpenJPEG would
/// refuse it ("unsupported bpno_plus_one >= 31").
pub(crate) fn empty_cblk_ok(b: &T1Block<'_>) -> bool {
    (b.roishift.wrapping_add(b.numbps) as i32) < 31
}

impl T1 {
    /// The decoded coefficients, `w` x `h` of the last code-block.
    pub fn data(&self) -> &[i32] {
        &self.work.data[..(self.w * self.h).min(BUF)]
    }

    /// `opj_t1_allocate_buffers`, on the decoder's fixed buffers: the
    /// coefficients cleared, the flags cleared with OpenJPEG's guard rows
    /// and the partial last stripe marked.
    fn allocate_buffers(&mut self, w: usize, h: usize) {
        self.work.data[..(w * h).min(BUF)].fill(0);
        let flags_stride = w + 2;
        let flags_height = h.div_ceil(4);
        let flagssize = ((flags_height + 2) * flags_stride).min(BUF);
        self.work.flags[..flagssize].fill(0);
        for x in 0..flags_stride {
            self.work.flags[x & MASK] = T1_PI_ALL;
            self.work.flags[((flags_height + 1) * flags_stride + x) & MASK] = T1_PI_ALL;
        }
        if !h.is_multiple_of(4) {
            let v = match h % 4 {
                1 => T1_PI_1 | T1_PI_2 | T1_PI_3,
                2 => T1_PI_2 | T1_PI_3,
                _ => T1_PI_3,
            };
            for x in 0..flags_stride {
                self.work.flags[(flags_height * flags_stride + x) & MASK] = v;
            }
        }
        self.w = w;
        self.h = h;
    }

    /// Decodes the code-block into the scratch ([`T1::data`]), as
    /// `opj_t1_decode_cblk`; false when OpenJPEG refuses it (bit-planes plus
    /// ROI shift at or above 31). `tile_data` holds the bytes the chunks
    /// name.
    pub fn decode_cblk(&mut self, b: &T1Block<'_>, tile_data: &[u8]) -> bool {
        let (w, h) = (b.w as usize, b.h as usize);
        if w * h > BUF || w == 0 || h == 0 {
            // (never from the layout: a code-block has 1 to 4096 samples)
            return false;
        }
        self.allocate_buffers(w, h);
        if !empty_cblk_ok(b) {
            return false;
        }
        let mut bpno_plus_one = b.roishift.wrapping_add(b.numbps) as i32;
        let mut passtype = 2u32;
        reset_contexts(&mut self.work.ctxs);
        if b.chunks.is_empty() {
            return true;
        }

        // The code-block's chunks gathered (OpenJPEG reads a single chunk
        // where it lies, in its own copy of the tile's data; the decode is
        // the same), then room for the sentinel, cleared.
        self.cblkdata.clear();
        for c in b.chunks {
            let Some(bytes) = tile_data.get(c.pos as usize..c.pos as usize + c.len as usize) else {
                return false;
            };
            self.cblkdata.extend_from_slice(bytes);
        }
        let seg_bytes: usize = b.segs.iter().map(|s| s.len as usize).sum();
        let room = self.cblkdata.len().max(seg_bytes) + EXTRA;
        self.cblkdata.resize(room, 0);

        let vsc = b.cblksty & CBLK_VSC != 0;
        let is_64 = w == 64 && h == 64;
        let mut index = 0usize;
        for seg in b.segs {
            // BYPASS mode
            let raw = bpno_plus_one <= b.numbps as i32 - 4 && passtype < 2 && b.cblksty & CBLK_LAZY != 0;
            let end = index + seg.len as usize;
            // opj_mqc_init_dec_common: an artificial 0xFF 0xFF marker at the
            // end of the segment, so that the byte reads stop on it; the
            // bytes it covers (the next segment's first) are put back after
            let backup = [self.cblkdata[end], self.cblkdata[end + 1]];
            self.cblkdata[end] = 0xFF;
            self.cblkdata[end + 1] = 0xFF;
            let mut mq = Mq {
                a: 0,
                c: 0,
                ct: 0,
                bp: index,
            };
            if !raw {
                // opj_mqc_init_dec: ISO 15444-1 C.3.5 (INITDEC)
                mq.c = if seg.len == 0 {
                    0xff << 16
                } else {
                    u32::from(self.cblkdata[index]) << 16
                };
                mq_bytein(&mut mq, &self.cblkdata);
                mq.c <<= 7;
                mq.ct -= 7;
                mq.a = 0x8000;
            }
            index = end;

            {
                let mut cd = Coder {
                    work: &mut self.work,
                    buf: &self.cblkdata,
                    zc: (b.orient as usize & 3) << 9,
                };
                let mut passno = 0;
                while passno < seg.real_num_passes && bpno_plus_one >= 1 {
                    let bpno = bpno_plus_one as u32;
                    match passtype {
                        0 => {
                            if is_64 && !raw {
                                sigpass_64(&mut cd, &mut mq, bpno, vsc);
                            } else {
                                sigpass_generic(&mut cd, &mut mq, bpno, vsc, w, h, raw);
                            }
                        }
                        1 => {
                            if is_64 && !raw {
                                refpass_64(&mut cd, &mut mq, bpno);
                            } else {
                                refpass_generic(&mut cd, &mut mq, bpno, w, h, raw);
                            }
                        }
                        _ => {
                            if is_64 {
                                clnpass_64(&mut cd, &mut mq, bpno, vsc);
                            } else {
                                clnpass_generic(&mut cd, &mut mq, bpno, vsc, w, h);
                            }
                            if b.cblksty & CBLK_SEGSYM != 0 {
                                cd.segsym(&mut mq);
                            }
                        }
                    }
                    if b.cblksty & CBLK_RESET != 0 && !raw {
                        reset_contexts(&mut cd.work.ctxs);
                    }
                    passtype += 1;
                    if passtype == 3 {
                        passtype = 0;
                        bpno_plus_one -= 1;
                    }
                    passno += 1;
                }
            }

            // opq_mqc_finish_dec: the bytes the sentinel covered
            self.cblkdata[end] = backup[0];
            self.cblkdata[end + 1] = backup[1];
        }
        true
    }

    /// The ROI shift undone on the decoded coefficients
    /// (`opj_t1_clbl_decode_processor`).
    pub fn unshift_roi(&mut self, roishift: u32) {
        if roishift == 0 {
            return;
        }
        let n = (self.w * self.h).min(BUF);
        let data = &mut self.work.data[..n];
        if roishift >= 31 {
            data.fill(0);
            return;
        }
        let thresh = 1i32 << roishift;
        for d in data {
            let val = *d;
            // abs() of INT_MIN stays INT_MIN, as in OpenJPEG
            let mag = val.wrapping_abs();
            if mag >= thresh {
                let mag = mag >> roishift;
                *d = if val < 0 { -mag } else { mag };
            }
        }
    }
}

/// Writes decoded coefficients (`w` x `h`) into a tile-component buffer at
/// `dst` (rows of `stride` samples), as `opj_t1_clbl_decode_processor` does
/// for whole-tile decoding: 5/3 coefficients halved (towards zero), 9/7 ones
/// multiplied by `0.5 * stepsize` as floats (their bits stored in the int32
/// buffer). `None` writes zeros (a code-block without data).
pub(crate) fn store(src: Option<&[i32]>, w: u32, h: u32, reversible: bool, stepsize: f32, dst: &mut [i32], stride: usize) {
    let (w, h) = (w as usize, h as usize);
    let Some(src) = src else {
        // a code-block without data: its coefficients are 0, in either representation
        for j in 0..h {
            dst[j * stride..j * stride + w].fill(0);
        }
        return;
    };
    if reversible {
        // positive -> round down: (83)/2 = 41.5 -> 41; negative -> round up: (-83)/2 = -41.5 -> -41
        for (j, row) in src.chunks_exact(w).take(h).enumerate() {
            for (o, &v) in dst[j * stride..j * stride + w].iter_mut().zip(row) {
                *o = v / 2;
            }
        }
    } else {
        let half_step = 0.5 * stepsize;
        for (j, row) in src.chunks_exact(w).take(h).enumerate() {
            for (o, &v) in dst[j * stride..j * stride + w].iter_mut().zip(row) {
                *o = (v as f32 * half_step).to_bits() as i32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mq_states_follow_table_c2() {
        // row 0: Qe 0x5601, NMPS 1, NLPS 1, switch 1
        use crate::t1_luts::MQC_STATES;
        assert_eq!(MQC_STATES[0] & 0xffff, 0x5601);
        assert_eq!((MQC_STATES[0] >> 16) & 0xff, 2);
        assert_eq!(MQC_STATES[0] >> 24, 3);
        assert_eq!(MQC_STATES[1] >> 24, 2);
        // row 46 (the uniform context): Qe 0x5601, stays
        assert_eq!(MQC_STATES[92] & 0xffff, 0x5601);
        assert_eq!((MQC_STATES[92] >> 16) & 0xff, 92);
        assert_eq!(MQC_STATES[92] >> 24, 92);
        // every next state is a state, and a context's word says what its state does
        for (i, st) in MQC_STATES.iter().enumerate().take(94) {
            assert!((st >> 16) & 0xff < 94 && st >> 24 < 94, "state {i}");
            let w = MQC_WORDS[i];
            assert_eq!(
                (w & 0xffff, (w >> 16) & 1, (w >> 17) & 0x7f, w >> 24),
                (st & 0xffff, i as u32 & 1, (st >> 16) & 0xff, st >> 24),
                "state {i}"
            );
        }
    }

    #[test]
    fn empty_block_decodes_to_zeros() {
        let mut t1 = T1::default();
        let b = T1Block {
            w: 5,
            h: 3,
            numbps: 7,
            orient: 0,
            roishift: 0,
            cblksty: 0,
            chunks: &[],
            segs: &[],
        };
        assert!(t1.decode_cblk(&b, &[]));
        assert_eq!(t1.data(), &[0; 15]);
        let too_deep = T1Block { numbps: 31, ..b };
        assert!(!t1.decode_cblk(&too_deep, &[]));
        assert!(!empty_cblk_ok(&too_deep));
    }

    #[test]
    fn store_halves_towards_zero_and_scales() {
        let src = [83, -83, 2, -1];
        let mut dst = [7; 6];
        store(Some(&src), 2, 2, true, 0.0, &mut dst, 3);
        assert_eq!(dst, [41, -41, 7, 1, 0, 7]);
        store(Some(&src), 2, 2, false, 0.5, &mut dst, 3);
        assert_eq!(f32::from_bits(dst[0] as u32), 20.75);
        assert_eq!(f32::from_bits(dst[4] as u32), -0.25);
        store(None, 2, 2, false, 0.5, &mut dst, 3);
        assert_eq!(dst, [0, 0, 7, 0, 0, 7]);
    }

    #[test]
    fn roi_unshift() {
        let mut t1 = T1::default();
        t1.allocate_buffers(2, 2);
        t1.work.data[..4].copy_from_slice(&[40, -40, 3, i32::MIN]);
        t1.unshift_roi(3);
        assert_eq!(t1.data(), &[5, -5, 3, i32::MIN]);
    }
}
